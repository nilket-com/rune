#[cfg(feature = "execution")]
use crate::TypeHash;
use core::fmt;

use crate as rune;
use crate::alloc::prelude::*;
use crate::alloc::{self, BTreeSet, Box, HashMap, HashSet, String, Vec};
#[cfg(feature = "emit")]
use crate::compile::MetaInfo;
use crate::compile::{self, ContextError, Names};
use crate::compile::{meta, Docs};
use crate::function::{Function, Plain};
use crate::function_meta::{AssociatedName, ToInstance};
use crate::hash;
use crate::item::{ComponentRef, IntoComponent};
use crate::macros::{MacroContext, TokenStream};
use crate::module::{
	DocFunction, Fields, Module, ModuleAssociated, ModuleAssociatedKind, ModuleFunction,
	ModuleItem, ModuleItemCommon, ModuleReexport, ModuleTrait, ModuleTraitImpl, ModuleType,
	TypeSpecification,
};
use crate::runtime::{
	Address, AnyTypeInfo, ConstConstructImpl, ConstContext, ConstValue, ConstValueBuf,
	FunctionHandler, Memory, Output, Protocol, Rtti, RttiKind, RuntimeContext, TypeInfo, VmError,
};
use crate::sync::Arc;
use crate::{Hash, Item, ItemBuf};

// Compile-time identities. Registration reuses existing effective hashes.
#[cfg(feature = "execution")]
const VEC_CONSTRUCTOR_RECEIVER: Hash = crate::hash_in!(crate, ::std::vec::Vec);
#[cfg(feature = "execution")]
const SORT_RECEIVER: Hash = crate::hash_in!(crate, ::std::vec::Vec);
#[cfg(feature = "execution")]
const SORT_ASSOCIATED: Hash = crate::hash_in!(crate, ::std::vec::Vec.sort_by);
#[cfg(feature = "execution")]
const SORT_ITEM: Hash = crate::hash_in!(crate, ::std::vec::Vec::sort_by);

// Mirror runtime/value.rs::{debug_fmt_with, display_fmt_with}: Inline debug
// bypasses protocols; scalar display does too. Any-backed values read both keys.
// The host importer rejects Any-backed primitive scalars before enrollment.
#[cfg(feature = "execution")]
pub(crate) fn formats_key(hash: Hash) -> bool {
	use crate::experimental_core::error::TypeName::*;
	[
		Unit,
		Ordering,
		String,
		Vec,
		Tuple,
		Object,
		Option,
		Result,
		Range,
		RangeInclusive,
		RangeFrom,
		RangeFull,
		RangeTo,
		RangeToInclusive,
		Function,
		Type,
		VecIter,
		ObjectIter,
		OptionIter,
		RangeIter,
		UnsignedRangeIter,
		CharRangeIter,
		RangeInclusiveIter,
		UnsignedRangeInclusiveIter,
		CharRangeInclusiveIter,
		RangeFromIter,
		UnsignedRangeFromIter,
		CharRangeFromIter,
		StringSplit,
		CharSplit,
		Chars,
		Skip,
		Take,
	]
	.into_iter()
	.any(|name| {
		name.builtin_hash().is_some_and(|receiver| {
			(!matches!(name, Unit | Ordering | Type)
				&& hash == Hash::associated_function(receiver, &Protocol::DEBUG_FMT))
				|| hash == Hash::associated_function(receiver, &Protocol::DISPLAY_FMT)
		})
	})
}

crate::declare_dyn_fn! {
	struct MacroHandlerVtable;

	/// A (type erased) macro handler.
	pub struct MacroHandler {
		fn call(cx: &mut MacroContext<'_, '_, '_>, input: &TokenStream) -> compile::Result<TokenStream>;
	}
}

crate::declare_dyn_fn! {
	struct TraitHandlerVtable;

	/// Invoked when types implement a trait.
	pub struct TraitHandler {
		fn call(cx: &mut TraitContext<'_>) -> Result<(), ContextError>;
	}
}

crate::declare_dyn_fn! {
	struct AttributeMacroHandlerVtable;

	/// A (type erased) attribute macro handler.
	pub struct AttributeMacroHandler {
		fn call(cx: &mut MacroContext<'_, '_, '_>, input: &TokenStream, attributes: &TokenStream) -> compile::Result<TokenStream>;
	}
}

/// Type used to install traits.
pub struct TraitContext<'a> {
	/// The context the trait function are being installed into.
	cx: &'a mut Context,
	/// The item being installed.
	item: &'a Item,
	/// The hash of the item being installed.
	hash: Hash,
	/// Type info of the type being installed.
	type_info: &'a TypeInfo,
	/// The trait being implemented for.
	trait_item: &'a Item,
	/// Hash of the trait being implemented.
	trait_hash: Hash,
}

impl TraitContext<'_> {
	/// Return the item the trait is being installed for.
	pub fn item(&self) -> &Item {
		self.item
	}

	/// Return the hash the trait is being installed for.
	pub fn hash(&self) -> Hash {
		self.hash
	}

	/// Find the given protocol function for the current type.
	///
	/// This requires that the function is defined.
	pub fn find(&mut self, protocol: &'static Protocol) -> Result<FunctionHandler, ContextError> {
		let name = protocol.to_instance()?;

		let hash = name
			.kind
			.hash(self.hash)
			.with_function_parameters(name.function_parameters);

		let Some(handler) = self.cx.functions.get(&hash) else {
			return Err(ContextError::MissingTraitFunction {
				name: name.kind.try_to_string()?,
				item: self.item.try_to_owned()?,
				hash,
				trait_item: self.trait_item.try_to_owned()?,
				trait_hash: self.trait_hash,
			});
		};

		let handler = handler.clone();

		if let Some(method) = protocol.method {
			#[cfg(feature = "execution")]
			let cmp_alias = if protocol.hash == Protocol::CMP.hash {
				self.cx.numeric.cmp_alias(self.hash)
			} else {
				None
			};
			#[cfg(feature = "execution")]
			let number_alias = self.cx.number.alias(protocol.hash, self.hash);
			self.function_handler(method, &handler)?;
			#[cfg(feature = "execution")]
			if let Some(pair) = cmp_alias {
				self.cx.numeric.certify(pair);
			}
			#[cfg(feature = "execution")]
			if let Some(pair) = number_alias {
				self.cx.number.forwarded(pair);
			}
		}

		Ok(handler)
	}

	/// Try to find the given associated function.
	///
	/// This does not require that the function is defined.
	pub fn try_find(&self, name: impl ToInstance) -> Result<Option<FunctionHandler>, ContextError> {
		let name = name.to_instance()?;

		let hash = name
			.kind
			.hash(self.hash)
			.with_function_parameters(name.function_parameters);

		Ok(self.cx.functions.get(&hash).cloned())
	}

	/// Find or define a protocol function.
	pub fn find_or_define<A, F>(
		&mut self,
		protocol: &'static Protocol,
		function: F,
	) -> Result<FunctionHandler, ContextError>
	where
		F: Function<A, Plain>,
	{
		let found = self.try_find(protocol)?;
		// Only a forward of the receiver's own protocol function is an alias.
		#[cfg(feature = "execution")]
		let number_alias = found
			.as_ref()
			.and_then(|_| self.cx.number.alias(protocol.hash, self.hash));
		let function = if let Some(function) = found {
			function
		} else {
			self.function(protocol, function)?
		};

		if let Some(method) = protocol.method {
			self.function_handler(method, &function)?;
			#[cfg(feature = "execution")]
			if let Some(pair) = number_alias {
				self.cx.number.forwarded(pair);
			}
		}

		Ok(function)
	}

	/// Define a new associated function for the current type.
	pub fn function<F, A>(
		&mut self,
		name: impl ToInstance,
		handler: F,
	) -> Result<FunctionHandler, ContextError>
	where
		F: Function<A, Plain>,
	{
		let handler = FunctionHandler::new(move |memory, addr, len, out| {
			handler.call(memory, addr, len, out)
		})?;
		self.function_handler(name, &handler)?;
		Ok(handler)
	}

	/// Define a new associated raw function for the current type.
	pub fn raw_function<F>(
		&mut self,
		name: impl ToInstance,
		handler: F,
	) -> Result<FunctionHandler, ContextError>
	where
		F: 'static
			+ Fn(&mut dyn Memory, Address, usize, Output) -> Result<(), VmError>
			+ Send
			+ Sync,
	{
		let handler = FunctionHandler::new(handler)?;
		self.function_handler(name, &handler)?;
		Ok(handler)
	}

	/// Define a new associated function for the current type using a raw
	/// handler.
	fn function_handler(
		&mut self,
		name: impl ToInstance,
		handler: &FunctionHandler,
	) -> Result<(), ContextError> {
		let name = name.to_instance()?;
		self.function_inner(name, handler)
	}

	fn function_inner(
		&mut self,
		name: AssociatedName,
		handler: &FunctionHandler,
	) -> Result<(), ContextError> {
		let function = ModuleFunction {
			#[cfg(feature = "execution")]
			format_default: None,
			#[cfg(feature = "execution")]
			host: None,
			handler: handler.clone(),
			trait_hash: Some(self.trait_hash),
			doc: DocFunction {
				#[cfg(feature = "doc")]
				is_async: false,
				#[cfg(feature = "doc")]
				args: None,
				#[cfg(feature = "doc")]
				argument_types: Box::default(),
				#[cfg(feature = "doc")]
				return_type: meta::DocType::empty(),
			},
		};

		let assoc = ModuleAssociated {
			container: self.hash,
			container_type_info: self.type_info.try_clone()?,
			name,
			common: ModuleItemCommon {
				docs: Docs::EMPTY,
				deprecated: None,
			},
			kind: ModuleAssociatedKind::Function(function),
		};

		self.cx.install_associated(&assoc)?;
		Ok(())
	}
}

/// Context metadata.
#[derive(Debug)]
#[non_exhaustive]
pub(crate) struct ContextMeta {
	/// Type hash for the given meta item.
	pub(crate) hash: Hash,
	/// The item of the returned compile meta.
	pub(crate) item: Option<ItemBuf>,
	/// The kind of the compile meta.
	pub(crate) kind: meta::Kind,
	/// Deprecation notice.
	#[cfg(feature = "doc")]
	pub(crate) deprecated: Option<Box<str>>,
	/// Documentation associated with a context meta.
	#[cfg(feature = "doc")]
	pub(crate) docs: Docs,
}

impl ContextMeta {
	#[cfg(feature = "emit")]
	pub(crate) fn info(&self) -> alloc::Result<MetaInfo> {
		MetaInfo::new(&self.kind, self.hash, self.item.as_deref())
	}
}

/// Information on a specific type.
#[derive(Debug, TryClone)]
#[non_exhaustive]
pub(crate) struct ContextType {
	/// Item of the type.
	item: ItemBuf,
	/// Type hash.
	hash: Hash,
	/// Complete detailed information on the hash.
	type_info: TypeInfo,
	/// Type parameters.
	type_parameters: Hash,
}

impl fmt::Display for ContextType {
	fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(fmt, "{} => {}", self.item, self.type_info)?;
		Ok(())
	}
}

/// [Context] used for the Rune language.
///
/// See [Build::with_context][crate::Build::with_context].
///
/// At runtime this needs to be converted into a [RuntimeContext] when used with
/// a [Vm][crate::runtime::Vm]. This is done through [Context::runtime].
///
/// A [Context] contains:
/// * Native functions.
/// * Native instance functions.
/// * And native type definitions.
#[derive(Default)]
pub struct Context {
	/// Unique modules installed in the context.
	unique: HashSet<&'static str>,
	/// Whether or not to include the prelude when constructing a new unit.
	has_default_modules: bool,
	/// Registered metadata, in the order that it was registered.
	meta: Vec<ContextMeta>,
	/// Item metadata in the context.
	hash_to_meta: hash::Map<Vec<usize>>,
	/// Store item to hash mapping.
	item_to_hash: HashMap<ItemBuf, BTreeSet<Hash>>,
	/// Registered native function handlers.
	functions: hash::Map<FunctionHandler>,
	#[cfg(feature = "execution")]
	hosts: hash::Map<crate::function::host::HostBinding>,
	#[cfg(feature = "execution")]
	format: crate::function::formatting::State,
	/// Registered deprecation mesages for native functions.
	deprecations: hash::Map<String>,
	/// Information on associated types.
	#[cfg(feature = "doc")]
	associated: hash::Map<Vec<Hash>>,
	/// Traits implemented by the given hash.
	#[cfg(feature = "doc")]
	implemented_traits: hash::Map<Vec<Hash>>,
	/// Registered native macro handlers.
	macros: hash::Map<MacroHandler>,
	/// Execution core: certified std formatting and assertion macros. A hash here implies that the handler stored
	/// in `macros` at that hash is the marked std builtin; a builtin may be absent from it
	/// after an allocation failure (fail closed).
	#[cfg(feature = "execution")]
	builtin_macros: HashSet<Hash>,
	/// Execution core: the handler installed at `::std::io::println` is the std stdio builtin.
	#[cfg(feature = "execution")]
	stdio_println: bool,
	#[cfg(feature = "execution")]
	stdio_print: bool,
	/// Execution core: this module/context certifies the std panic builtin.
	#[cfg(feature = "execution")]
	panic: bool,
	/// Execution core: this module/context certifies String::from.
	#[cfg(feature = "execution")]
	string_from: bool,
	#[cfg(feature = "execution")]
	vec_constructors: u8,
	#[cfg(feature = "execution")]
	vec_resize: bool,
	#[cfg(feature = "execution")]
	clone: u8,
	#[cfg(feature = "execution")]
	numeric: crate::function::numeric::State,
	#[cfg(feature = "execution")]
	sort_by: bool,
	#[cfg(feature = "execution")]
	sort_by_closed: bool,
	#[cfg(feature = "execution")]
	iter: crate::function::iter::State,
	#[cfg(feature = "execution")]
	text: crate::function::text::State,
	#[cfg(feature = "execution")]
	pipeline: crate::function::pipeline::State,
	#[cfg(feature = "execution")]
	variant: crate::function::variant::State,
	#[cfg(feature = "execution")]
	object: crate::function::object::State,
	#[cfg(feature = "execution")]
	number: crate::function::number::State,
	#[cfg(feature = "execution")]
	methods: crate::function::methods::State,
	/// Handlers for realizing traits.
	traits: hash::Map<Option<TraitHandler>>,
	/// Registered native attribute macro handlers.
	attribute_macros: hash::Map<AttributeMacroHandler>,
	/// Registered types.
	types: hash::Map<ContextType>,
	/// All available names in the context.
	names: Names,
	/// Registered crates.
	crates: HashSet<Box<str>>,
	/// Constants visible in this context
	constants: hash::Map<ConstValueBuf>,
	/// Constant constructor.
	construct: hash::Map<ConstConstructImpl>,
}

// Only with_config owns this guard. Its factory list accepts only the stdio
// boolean and calls Rune's closed std factories: no embedder callback can run
// while exact native insertions inherit this private std provenance.
#[cfg(feature = "execution")]
struct StdFormatEpoch<'a> {
	context: &'a mut Context,
}
#[cfg(feature = "execution")]
impl<'a> StdFormatEpoch<'a> {
	fn enter(context: &'a mut Context) -> Self {
		assert!(
			!context.format.trusted_install,
			"default formatter installation epoch is not reentrant"
		);
		context.format.trusted_install = true;
		Self { context }
	}
}
#[cfg(feature = "execution")]
impl core::ops::Deref for StdFormatEpoch<'_> {
	type Target = Context;
	fn deref(&self) -> &Context {
		self.context
	}
}
#[cfg(feature = "execution")]
impl core::ops::DerefMut for StdFormatEpoch<'_> {
	fn deref_mut(&mut self) -> &mut Context {
		self.context
	}
}
#[cfg(feature = "execution")]
impl Drop for StdFormatEpoch<'_> {
	fn drop(&mut self) {
		self.context.format.trusted_install = false;
	}
}

impl Context {
	#[cfg(feature = "execution")]
	pub(crate) fn formatting(&self) -> bool {
		!self.format.modified
	}

	#[cfg(feature = "execution")]
	pub(crate) fn format_snapshot(
		&self,
	) -> Option<crate::sync::Arc<crate::function::formatting::Certificate>> {
		self.format.snapshot()
	}
	#[cfg(feature = "execution")]
	pub(crate) fn permits_nominal_format(&self, hash: Hash) -> bool {
		self.format.permits(hash)
	}

	/// Construct a new empty [Context].
	#[inline]
	pub fn new() -> Self {
		Self::default()
	}

	/// Construct a [Context] containing the default set of modules with the
	/// given configuration.
	///
	/// `stdio` determines if we include I/O functions that interact with stdout
	/// and stderr by default, like `dbg`, `print`, and `println`. If this is
	/// `false` all the corresponding low-level I/O functions have to be
	/// provided through a different module.
	///
	/// These are:
	///
	/// * `::std::io::dbg`
	/// * `::std::io::print`
	/// * `::std::io::println`
	pub fn with_config(#[allow(unused)] stdio: bool) -> Result<Self, ContextError> {
		let mut this = Self::new();
		{
			#[cfg(feature = "execution")]
			let mut epoch = StdFormatEpoch::enter(&mut this);
			#[cfg(feature = "execution")]
			let this = &mut *epoch;
			#[cfg(not(feature = "execution"))]
			let this = &mut this;

			// NB: Order is important, since later modules might use types defined
			// in previous modules.

			this.install(crate::modules::iter::module()?)?;
			this.install(crate::modules::core::module()?)?;
			this.install(crate::modules::cmp::module()?)?;
			this.install(crate::modules::any::module()?)?;
			this.install(crate::modules::clone::module()?)?;
			this.install(crate::modules::num::module()?)?;
			this.install(crate::modules::hash::module()?)?;

			this.install(crate::modules::string::module()?)?;
			this.install(crate::modules::bytes::module()?)?;

			this.install(crate::modules::collections::module()?)?;
			this.install(crate::modules::collections::hash_map::module()?)?;
			this.install(crate::modules::collections::hash_set::module()?)?;
			this.install(crate::modules::collections::vec_deque::module()?)?;

			this.install(crate::modules::char::module()?)?;
			this.install(crate::modules::f64::module()?)?;
			this.install(crate::modules::f64::consts::module()?)?;
			this.install(crate::modules::tuple::module()?)?;
			this.install(crate::modules::fmt::module()?)?;
			this.install(crate::modules::future::module()?)?;
			this.install(crate::modules::i64::module()?)?;
			this.install(crate::modules::u64::module()?)?;
			this.install(crate::modules::io::module(stdio)?)?;
			this.install(crate::modules::macros::module()?)?;
			this.install(crate::modules::macros::builtin::module()?)?;
			this.install(crate::modules::mem::module()?)?;
			this.install(crate::modules::object::module()?)?;
			this.install(crate::modules::ops::module()?)?;
			this.install(crate::modules::ops::generator::module()?)?;
			this.install(crate::modules::option::module()?)?;
			this.install(crate::modules::result::module()?)?;
			this.install(crate::modules::stream::module()?)?;
			this.install(crate::modules::test::module()?)?;
			this.install(crate::modules::vec::module()?)?;
			this.install(crate::modules::slice::module()?)?;
			this.has_default_modules = true;
		}
		Ok(this)
	}

	/// Construct a new collection of functions with default packages installed.
	pub fn with_default_modules() -> Result<Self, ContextError> {
		Self::with_config(true)
	}

	/// Construct a runtime context used when executing the virtual machine.
	///
	/// This is not a cheap operation, since it requires cloning things out of
	/// the build-time [Context] which are necessary at runtime.
	///
	/// ```no_run
	/// use rune::{Context, Vm, Unit};
	/// use rune::sync::Arc;
	///
	/// let context = Context::with_default_modules()?;
	///
	/// let runtime = Arc::try_new(context.runtime()?)?;
	/// let unit = Arc::try_new(Unit::default())?;
	///
	/// let vm = Vm::new(runtime, unit);
	/// # Ok::<_, rune::support::Error>(())
	/// ```
	pub fn runtime(&self) -> alloc::Result<RuntimeContext> {
		Ok(RuntimeContext::new(
			self.functions.try_clone()?,
			self.constants.try_clone()?,
			self.construct.try_clone()?,
			self.deprecations.try_clone()?,
			#[cfg(feature = "execution")]
			self.format.snapshot(),
		))
	}

	/// Install the specified module.
	///
	/// This installs everything that has been declared in the given [Module]
	/// and ensures that they are compatible with the overall context, like
	/// ensuring that a given type is only declared once.
	#[tracing::instrument(skip_all, fields(item = ?module.as_ref().item))]
	pub fn install<M>(&mut self, module: M) -> Result<(), ContextError>
	where
		M: AsRef<Module>,
	{
		let module = module.as_ref();
		tracing::trace!("installing");

		if let Some(id) = module.unique {
			if !self.unique.try_insert(id)? {
				return Ok(());
			}
		}

		#[cfg(feature = "execution")]
		let (numeric_pending, sort_builtin) = {
			use crate::function::numeric::{Method, Pair};
			let mut methods = 0u8;
			let mut pairs = 0u8;
			let sort_candidate = module.sort_by && !self.sort_by_closed && !self.sort_by;
			let mut sort_touches = false;
			let mut sort_names = 0usize;
			let mut sort_trusted = false;
			let mut sort_item_collision = false;
			for assoc in &module.associated {
				if matches!(&assoc.name.kind, meta::AssociatedKind::Protocol(p) if p.hash == Protocol::CMP.hash)
					&& Method::Cmp.closes_for(assoc.container)
				{
					methods |= Method::Cmp.bit();
				}
				let effective = assoc
					.name
					.kind
					.hash(assoc.container)
					.with_function_parameters(assoc.name.function_parameters);
				let named_sort = matches!(&assoc.name.kind, meta::AssociatedKind::Instance(n) if n.as_ref() == "sort_by");
				sort_touches |= named_sort || effective == SORT_ASSOCIATED;
				if named_sort {
					sort_names = sort_names.saturating_add(1);
					sort_trusted |= assoc.container == SORT_RECEIVER
						&& assoc.name.function_parameters == Hash::EMPTY
						&& matches!(&assoc.kind, ModuleAssociatedKind::Function(_));
				}
				for m in [Method::Abs, Method::Round, Method::Powf, Method::Cmp] {
					if (matches!(&assoc.name.kind,meta::AssociatedKind::Instance(n) if n.as_ref()==m.name())
						|| effective
							== Hash::associated_function(
								assoc.container,
								self.numeric.name_hash(m),
							)) && m.closes_for(assoc.container)
					{
						methods |= m.bit();
					}
				}
				for p in Pair::ALL {
					if effective == p.associated() {
						pairs |= p.bit();
					}
				}
			}
			for item in &module.items {
				sort_item_collision |= item.hash == SORT_ASSOCIATED || item.hash == SORT_ITEM;
				for p in Pair::ALL {
					if item.hash == p.item() || item.hash == p.associated() {
						pairs |= p.bit();
						methods |= p.method().bit();
					}
				}
			}
			if sort_touches || sort_item_collision {
				self.sort_by = false;
				self.sort_by_closed = true;
			}
			let sort_builtin =
				sort_candidate && !sort_item_collision && sort_names == 1 && sort_trusted;
			(self.numeric.begin_module(methods, pairs), sort_builtin)
		};
		#[cfg(feature = "execution")]
		let iter_pending = self.iter.begin_module(module.iter);
		#[cfg(feature = "execution")]
		let text_pending = self.text.begin_module(module.text);
		#[cfg(feature = "execution")]
		let pipeline_pending = self.pipeline.begin_module(module.pipeline);
		#[cfg(feature = "execution")]
		let variant_pending = self.variant.begin_module(module.variant);
		#[cfg(feature = "execution")]
		let object_pending = self.object.begin_module(module.object);
		#[cfg(feature = "execution")]
		let number_pending = self.number.begin_module(module.number);
		if let Some(ComponentRef::Crate(name)) = module.item.first() {
			self.crates.try_insert(name.try_into()?)?;
		}

		#[cfg(feature = "execution")]
		let prepared_hosts = {
			let mut hosts = self.hosts.try_clone()?;
			for item in &module.items {
				if let rune::module::ModuleItemKind::Function(function) = &item.kind {
					if let Some(enrollment) = function.host.filter(|e| e.admitted()) {
						hosts.try_insert(
							item.hash,
							crate::function::host::HostBinding {
								hash: item.hash,
								handler: function.handler.clone(),
								signature: enrollment.signature,
								certificate: enrollment.certificate,
							},
						)?;
					}
				}
			}
			hosts
		};
		tracing::trace!("module");
		self.install_module(module)?;

		tracing::trace!(types = module.types.len(), "types");
		for ty in &module.types {
			self.install_type(ty)?;
		}

		tracing::trace!(traits = module.traits.len(), "traits");
		for t in &module.traits {
			self.install_trait(t)?;
		}

		tracing::trace!(items = module.items.len(), "items");
		for item in &module.items {
			self.install_item(item)?;
			// Each private marker belongs to the std module registration. Certify only
			// after this exact item was installed. Native functions are insert-once.
			#[cfg(feature = "execution")]
			if matches!(item.kind, rune::module::ModuleItemKind::Function(..))
				&& module.panic
				&& item.hash == Hash::type_hash(crate::item!(::std::panic))
			{
				self.panic = true;
			}
			// Certified only after this item's handler was inserted: functions are
			// insert-once (ConflictingFunction), so the stored handler is this one.
			#[cfg(feature = "execution")]
			if module.stdio_println
				&& item.hash == Hash::type_hash(crate::item!(::std::io::println))
				&& matches!(item.kind, rune::module::ModuleItemKind::Function(..))
			{
				self.stdio_println = true;
			}
			#[cfg(feature = "execution")]
			if module.stdio_print
				&& item.hash == Hash::type_hash(crate::item!(::std::io::print))
				&& matches!(item.kind, rune::module::ModuleItemKind::Function(..))
			{
				self.stdio_print = true;
			}
		}

		tracing::trace!(associated = module.associated.len(), "associated");
		for assoc in &module.associated {
			self.install_associated(assoc)?;
			#[cfg(feature = "execution")]
			if (module.clone != 0 || module.vec_resize)
				&& assoc.name.function_parameters == Hash::EMPTY
				&& matches!(&assoc.kind, ModuleAssociatedKind::Function(f) if f.trait_hash.is_none())
			{
				if module.clone != 0
					&& matches!(&assoc.name.kind, meta::AssociatedKind::Protocol(p) if p.hash == rune::runtime::Protocol::CLONE.hash)
				{
					if let Some(provider) =
						crate::function::clone::Provider::from_receiver(assoc.container)
					{
						self.clone |= module.clone & provider.bit();
					}
				}
				if module.vec_resize
					&& assoc.container == crate::function::clone::Provider::Vec.receiver()
					&& matches!(&assoc.name.kind, meta::AssociatedKind::Instance(name) if name.as_ref() == "resize")
				{
					self.vec_resize = true;
				}
			}

			#[cfg(feature = "execution")]
			for pair in crate::function::numeric::Pair::ALL {
				if module.numeric & pair.bit() != 0
					&& pair.cmp_protocol().is_some()
					&& assoc.container == pair.receiver()
					&& assoc.name.function_parameters == Hash::EMPTY
					&& matches!(&assoc.name.kind, meta::AssociatedKind::Protocol(p) if p.hash == Protocol::CMP.hash)
					&& matches!(&assoc.kind, ModuleAssociatedKind::Function(_))
				{
					self.numeric.certify_cmp_protocol(pair);
				}
				if module.numeric & pair.bit() != 0
					&& assoc.container == pair.receiver()
					&& assoc.name.function_parameters == Hash::EMPTY
					&& matches!(&assoc.name.kind, meta::AssociatedKind::Instance(n) if n.as_ref()==pair.method().name())
					&& matches!(&assoc.kind, ModuleAssociatedKind::Function(_))
				{
					self.numeric.certify(pair);
				}
			}
			#[cfg(feature = "execution")]
			if module.vec_constructors
				&& assoc.container == VEC_CONSTRUCTOR_RECEIVER
				&& assoc.name.function_parameters == Hash::EMPTY
				&& matches!(&assoc.kind, ModuleAssociatedKind::Function(f) if f.trait_hash.is_none())
			{
				if let meta::AssociatedKind::Instance(name) = &assoc.name.kind {
					self.vec_constructors |= match name.as_ref() {
						"new" => 1,
						"with_capacity" => 2,
						_ => 0,
					};
				}
			}
			// String::from is registered as a named type item even with `free`.
			// install_associated inserts its static item hash and associated hash
			// before returning; a conflict on either leaves it uncertified.
			#[cfg(feature = "execution")]
			if module.string_from
				&& assoc.container == Hash::type_hash(crate::item!(::std::string::String))
				&& assoc.name.function_parameters == Hash::EMPTY
				&& matches!(&assoc.name.kind, meta::AssociatedKind::Instance(name) if name.as_ref() == "from")
				&& matches!(assoc.kind, ModuleAssociatedKind::Function(..))
			{
				self.string_from = true;
			}
		}

		tracing::trace!(trait_impls = module.trait_impls.len(), "trait impls");
		for t in &module.trait_impls {
			self.install_trait_impl(t)?;
		}
		#[cfg(feature = "execution")]
		if module.number != 0 {
			self.number.certify_module(module);
		}

		tracing::trace!(reexports = module.reexports.len(), "reexports");
		for r in &module.reexports {
			self.install_reexport(r)?;
		}

		tracing::trace!(construct = module.construct.len(), "construct");
		for (hash, type_info, construct) in &module.construct {
			self.install_construct(*hash, type_info, construct)?;
		}

		#[cfg(feature = "execution")]
		{
			self.hosts = prepared_hosts;
			self.numeric.finish_module(numeric_pending);
			self.iter.finish_module(iter_pending);
			self.text.finish_module(text_pending);
			self.pipeline.finish_module(pipeline_pending);
			self.variant.finish_module(variant_pending);
			self.object.finish_module(object_pending);
			self.number.finish_module(number_pending);
			if sort_builtin {
				self.sort_by = true;
				self.sort_by_closed = false;
			}
		}
		Ok(())
	}

	/// Iterate over all available functions in the [Context].
	#[cfg(any(feature = "cli", feature = "languageserver"))]
	pub(crate) fn iter_functions(&self) -> impl Iterator<Item = (&ContextMeta, &meta::Signature)> {
		self.meta.iter().flat_map(|meta| {
			let signature = meta.kind.as_signature()?;
			Some((meta, signature))
		})
	}

	/// Iterate over all available types in the [Context].
	#[cfg(feature = "cli")]
	pub(crate) fn iter_types(&self) -> impl Iterator<Item = (Hash, &Item)> {
		use core::iter;

		let mut it = self.types.iter();

		iter::from_fn(move || {
			let (hash, ty) = it.next()?;
			Some((*hash, ty.item.as_ref()))
		})
	}

	/// Iterate over known child components of the given name.
	pub(crate) fn iter_components<'a, I>(
		&'a self,
		iter: I,
	) -> alloc::Result<impl Iterator<Item = ComponentRef<'a>> + 'a>
	where
		I: 'a + IntoIterator<Item: IntoComponent>,
	{
		self.names.iter_components(iter)
	}

	/// Access the context meta for the given item.
	///
	/// If this returns `Some`, at least one context meta is guaranteed to be
	/// available.
	pub(crate) fn lookup_meta(
		&self,
		item: &Item,
	) -> Option<impl Iterator<Item = &ContextMeta> + Clone> {
		let hashes = self.item_to_hash.get(item)?;

		Some(hashes.iter().flat_map(|hash| {
			let indexes = self
				.hash_to_meta
				.get(hash)
				.map(Vec::as_slice)
				.unwrap_or_default();
			indexes.iter().map(|&i| &self.meta[i])
		}))
	}

	/// Lookup meta by its hash.
	#[cfg(any(
		feature = "cli",
		feature = "languageserver",
		feature = "emit",
		feature = "execution"
	))]
	pub(crate) fn lookup_meta_by_hash(
		&self,
		hash: Hash,
	) -> impl ExactSizeIterator<Item = &ContextMeta> + Clone {
		let indexes = self
			.hash_to_meta
			.get(&hash)
			.map(Vec::as_slice)
			.unwrap_or_default();

		indexes.iter().map(|&i| &self.meta[i])
	}

	/// Lookup deprecation by function hash.
	pub fn lookup_deprecation(&self, hash: Hash) -> Option<&str> {
		self.deprecations.get(&hash).map(|s| s.as_str())
	}

	/// Check if unit contains the given name by prefix.
	pub(crate) fn contains_prefix(&self, item: &Item) -> alloc::Result<bool> {
		self.names.contains_prefix(item)
	}

	#[cfg(feature = "execution")]
	pub(crate) fn host(&self, hash: Hash) -> Option<&crate::function::host::HostBinding> {
		let binding = self.hosts.get(&hash)?;
		let handler = self.functions.get(&hash)?;
		binding.handler.same_allocation(handler).then_some(binding)
	}

	// Unit-test-only synthetic certificate for raw VM error controls. Never exported.
	#[cfg(all(test, feature = "execution"))]
	pub(crate) fn host_for_test(
		&mut self,
		hash: Hash,
		signature: crate::function::host::HostSignature,
	) -> alloc::Result<()> {
		assert!(signature.valid());
		let handler = self
			.functions
			.get(&hash)
			.expect("installed test handler")
			.clone();
		self.hosts.try_insert(
			hash,
			crate::function::host::HostBinding {
				hash,
				handler,
				signature,
				certificate: None,
			},
		)?;
		Ok(())
	}

	#[cfg(feature = "execution")]
	pub(crate) fn method_closed(&self, name: Hash) -> bool {
		self.methods.closed(name)
	}

	/// Every type the context registers, by type hash.
	#[cfg(feature = "execution")]
	pub(crate) fn types(&self) -> impl Iterator<Item = Hash> + '_ {
		self.types.keys().copied()
	}

	/// Lookup the given native function handler in the context.
	pub(crate) fn lookup_function(&self, hash: Hash) -> Option<&FunctionHandler> {
		self.functions.get(&hash)
	}

	/// Get all associated types for the given hash.
	#[cfg(all(feature = "doc", feature = "cli"))]
	pub(crate) fn associated(&self, hash: Hash) -> impl Iterator<Item = Hash> + '_ {
		self.associated
			.get(&hash)
			.map(Vec::as_slice)
			.unwrap_or_default()
			.iter()
			.copied()
	}

	/// Get all traits implemented for the given hash.
	#[cfg(all(feature = "doc", feature = "cli"))]
	pub(crate) fn traits(&self, hash: Hash) -> impl Iterator<Item = Hash> + '_ {
		self.implemented_traits
			.get(&hash)
			.map(Vec::as_slice)
			.unwrap_or_default()
			.iter()
			.copied()
	}

	#[cfg(feature = "execution")]
	pub(crate) fn builtin_macro(&self, hash: Hash) -> bool {
		self.builtin_macros.contains(&hash)
			&& (hash == Hash::type_hash(crate::item!(::std::fmt::format))
				|| hash == Hash::type_hash(crate::item!(::std::io::println))
				|| hash == Hash::type_hash(crate::item!(::std::io::print))
				|| hash == Hash::type_hash(crate::item!(::std::panic))
				|| hash == Hash::type_hash(crate::item!(::std::test::assert))
				|| hash == Hash::type_hash(crate::item!(::std::test::assert_eq))
				|| hash == Hash::type_hash(crate::item!(::std::test::assert_ne)))
	}

	/// Execution core: whether `::std::io::println` resolves to the certified std
	/// stdio builtin (not capture, disable, custom or absent).
	#[cfg(feature = "execution")]
	pub(crate) fn stdio_println(&self) -> bool {
		self.stdio_println
			&& self
				.functions
				.contains_key(&Hash::type_hash(crate::item!(::std::io::println)))
	}

	#[cfg(feature = "execution")]
	pub(crate) fn stdio_print(&self) -> bool {
		self.stdio_print
			&& self
				.functions
				.contains_key(&Hash::type_hash(crate::item!(::std::io::print)))
	}

	#[cfg(feature = "execution")]
	pub(crate) fn panic(&self) -> bool {
		self.panic
			&& self
				.functions
				.contains_key(&Hash::type_hash(crate::item!(::std::panic)))
	}

	#[cfg(feature = "execution")]
	pub(crate) fn clone_provider(&self, provider: crate::function::clone::Provider) -> bool {
		self.clone & provider.bit() != 0 && self.functions.contains_key(&provider.protocol())
	}

	#[cfg(feature = "execution")]
	pub(crate) fn vec_resize(&self) -> bool {
		self.vec_resize && self.functions.contains_key(&crate::function::clone::RESIZE)
	}

	#[cfg(feature = "execution")]
	pub(crate) fn vec_constructor(&self, with_capacity: bool) -> bool {
		let bit = if with_capacity { 2 } else { 1 };
		let hash = if with_capacity {
			crate::hash_in!(crate, ::std::vec::Vec::with_capacity)
		} else {
			crate::hash_in!(crate, ::std::vec::Vec::new)
		};
		self.vec_constructors & bit != 0 && self.functions.contains_key(&hash)
	}

	#[cfg(feature = "execution")]
	pub(crate) fn string_from(&self) -> bool {
		self.string_from
			&& self
				.functions
				.contains_key(&Hash::type_hash(crate::item!(::std::string::String::from)))
	}
	#[cfg(feature = "execution")]
	pub(crate) fn sort_by(&self) -> bool {
		self.sort_by && !self.sort_by_closed
	}

	/// Execution core (C12): `name` resolves to the certified std providers for every receiver a
	/// new value can have. Script methods of the same name are checked by the lowerer.
	#[cfg(feature = "execution")]
	pub(crate) fn any_function(&self, h: Hash) -> bool {
		self.pipeline.any_ready(h)
	}
	#[cfg(feature = "execution")]
	pub(crate) fn pipeline_method(&self, m: crate::function::pipeline::Method) -> bool {
		self.pipeline.ready(m)
	}
	#[cfg(feature = "execution")]
	pub(crate) fn text_method(&self, m: crate::function::text::Method) -> bool {
		self.text.ready(m)
	}
	/// Execution core (C17a): every stock provider of `m` is the certified std registration.
	#[cfg(feature = "execution")]
	pub(crate) fn variant_method(&self, m: crate::function::variant::Method) -> bool {
		self.variant.ready(m)
	}
	/// Execution core (C17c): the stock provider `pair` is its std registration.
	#[cfg(feature = "execution")]
	pub(crate) fn number_pair(&self, pair: crate::function::number::Pair) -> bool {
		self.number.ready(pair)
	}
	/// Execution core (C17c): every stock provider of `m` is its std registration.
	#[cfg(feature = "execution")]
	pub(crate) fn number_method(&self, m: crate::function::number::Method) -> bool {
		self.number.method_ready(m)
	}
	/// Execution core (C17b): every certified provider of `m` is its std registration.
	#[cfg(feature = "execution")]
	pub(crate) fn object_method(&self, m: crate::function::object::Method) -> bool {
		self.object.ready(m)
	}
	#[cfg(feature = "execution")]
	pub(crate) fn text_collect(&self) -> bool {
		self.text.collect_ready()
	}
	#[cfg(feature = "execution")]
	pub(crate) fn iter_method(&self, name: crate::function::iter::Name) -> bool {
		self.iter.certified(name)
	}
	#[cfg(feature = "execution")]
	pub(crate) fn numeric_method(&self, method: crate::function::numeric::Method) -> bool {
		self.numeric.method(method)
	}
	#[cfg(feature = "execution")]
	pub(crate) fn numeric_direct(&self, pair: crate::function::numeric::Pair) -> bool {
		self.numeric.direct(pair)
	}

	/// Lookup the given macro handler.
	pub(crate) fn lookup_macro(&self, hash: Hash) -> Option<&MacroHandler> {
		self.macros.get(&hash)
	}

	/// Lookup the given attribute macro handler.
	pub(crate) fn lookup_attribute_macro(&self, hash: Hash) -> Option<&AttributeMacroHandler> {
		self.attribute_macros.get(&hash)
	}

	/// Iterate over available crates.
	#[cfg(feature = "cli")]
	pub(crate) fn iter_crates(&self) -> impl Iterator<Item = &str> {
		self.crates.iter().map(|s| s.as_ref())
	}

	/// Check if context contains the given crate.
	pub(crate) fn contains_crate(&self, name: &str) -> bool {
		self.crates.contains(name)
	}

	/// Test if the context has the default modules installed.
	///
	/// This determines among other things whether a prelude should be used or
	/// not.
	pub(crate) fn has_default_modules(&self) -> bool {
		self.has_default_modules
	}

	/// Try to find an existing module.
	fn find_existing_module(&self, hash: Hash) -> Option<usize> {
		let indexes = self.hash_to_meta.get(&hash)?;

		for &index in indexes {
			let Some(m) = self.meta.get(index) else {
				continue;
			};

			if matches!(m.kind, meta::Kind::Module) {
				return Some(index);
			}
		}

		None
	}

	/// Install the given meta.
	#[tracing::instrument(skip_all)]
	fn install_meta(&mut self, meta: ContextMeta) -> Result<(), ContextError> {
		if let Some(item) = &meta.item {
			tracing::trace!(?item);

			self.names.insert(item)?;

			self.item_to_hash
				.entry(item.try_clone()?)
				.or_try_default()?
				.try_insert(meta.hash)?;
		}

		#[cfg(feature = "doc")]
		if let Some(h) = meta.kind.associated_container() {
			let assoc = self.associated.entry(h).or_try_default()?;
			assoc.try_push(meta.hash)?;
		}

		let hash = meta.hash;

		let index = self.meta.len();

		self.meta.try_push(meta)?;

		self.hash_to_meta
			.entry(hash)
			.or_try_default()?
			.try_push(index)?;

		Ok(())
	}

	/// Install a module, ensuring that its meta is defined.
	fn install_module(&mut self, m: &Module) -> Result<(), ContextError> {
		self.names.insert(&m.item)?;

		let mut current = Some((m.item.as_ref(), Some(&m.common)));

		#[allow(unused)]
		while let Some((item, common)) = current.take() {
			let hash = Hash::type_hash(item);

			if let Some(index) = self.find_existing_module(hash) {
				#[cfg(feature = "doc")]
				if let Some(common) = common {
					let meta = &mut self.meta[index];
					meta.deprecated = common.deprecated.try_clone()?;
					meta.docs = common.docs.try_clone()?;
				}
			} else {
				self.install_meta(ContextMeta {
					hash,
					item: Some(item.try_to_owned()?),
					kind: meta::Kind::Module,
					#[cfg(feature = "doc")]
					deprecated: common
						.map(|c| c.deprecated.as_ref().try_cloned())
						.transpose()?
						.flatten(),
					#[cfg(feature = "doc")]
					docs: common
						.map(|c| c.docs.try_clone())
						.transpose()?
						.unwrap_or_default(),
				})?;
			}

			current = item.parent().map(|item| (item, None));
		}

		Ok(())
	}

	/// Install a single type.
	fn install_type(&mut self, ty: &ModuleType) -> Result<(), ContextError> {
		self.install_type_info(ContextType {
			item: ty.item.try_to_owned()?,
			hash: ty.hash,
			type_info: ty.type_info.try_clone()?,
			type_parameters: ty.type_parameters,
		})?;

		let parameters = Hash::EMPTY.with_type_parameters(ty.type_parameters);

		let kind = if let Some(spec) = &ty.spec {
			match spec {
				TypeSpecification::Struct(fields) => {
					let constructor = match &ty.constructor {
						Some(c) => {
							let signature = meta::Signature {
								#[cfg(feature = "doc")]
								is_async: false,
								#[cfg(feature = "doc")]
								arguments: Some(fields_to_arguments(fields)?),
								#[cfg(feature = "doc")]
								return_type: meta::DocType::new(ty.hash),
							};

							if c.args != fields.len() {
								return Err(ContextError::ConstructorArgumentsMismatch {
									type_info: ty.type_info.try_clone()?,
									expected: fields.len(),
									actual: c.args,
								});
							}

							self.insert_native_fn(
								&ty.type_info,
								ty.hash,
								&c.handler,
								None,
								#[cfg(feature = "execution")]
								false,
							)?;
							Some(signature)
						}
						None => None,
					};

					meta::Kind::Struct {
						fields: match fields {
							Fields::Named(fields) => meta::Fields::Named(meta::FieldsNamed {
								fields: fields
									.iter()
									.copied()
									.enumerate()
									.map(|(position, name)| {
										Ok(meta::FieldMeta {
											name: name.try_into()?,
											position,
										})
									})
									.try_collect::<alloc::Result<_>>()??,
							}),
							Fields::Unnamed(args) => meta::Fields::Unnamed(*args),
							Fields::Empty => meta::Fields::Empty,
						},
						constructor,
						parameters,
						enum_hash: Hash::EMPTY,
					}
				}
				TypeSpecification::Enum(en) => {
					for variant in &en.variants {
						let Some(fields) = &variant.fields else {
							continue;
						};

						let kind = match fields {
							Fields::Empty => RttiKind::Empty,
							Fields::Unnamed(..) => RttiKind::Tuple,
							Fields::Named(..) => RttiKind::Struct,
						};

						let item = ty.item.extended(variant.name)?;
						let hash = Hash::type_hash(&item);

						self.install_type_info(ContextType {
							item: item.try_clone()?,
							hash,
							type_info: TypeInfo::rtti(Arc::try_new(Rtti {
								variant_index: 0,
								kind,
								hash: ty.hash,
								variant_hash: hash,
								item: item.try_clone()?,
								fields: fields.to_fields()?,
							})?),
							type_parameters: Hash::EMPTY,
						})?;

						let constructor = if let Some(c) = &variant.constructor {
							let signature = meta::Signature {
								#[cfg(feature = "doc")]
								is_async: false,
								#[cfg(feature = "doc")]
								arguments: Some(fields_to_arguments(fields)?),
								#[cfg(feature = "doc")]
								return_type: meta::DocType::new(ty.hash),
							};

							if c.args != fields.len() {
								return Err(ContextError::VariantConstructorArgumentsMismatch {
									type_info: ty.type_info.try_clone()?,
									name: variant.name,
									expected: fields.len(),
									actual: c.args,
								});
							}

							self.insert_native_fn(
								&item,
								hash,
								&c.handler,
								variant.deprecated.as_deref(),
								#[cfg(feature = "execution")]
								false,
							)?;
							Some(signature)
						} else {
							None
						};

						self.install_meta(ContextMeta {
							hash,
							item: Some(item),
							kind: meta::Kind::Struct {
								fields: match fields {
									Fields::Named(names) => {
										meta::Fields::Named(meta::FieldsNamed {
											fields: names
												.iter()
												.copied()
												.enumerate()
												.map(|(position, name)| {
													Ok(meta::FieldMeta {
														name: name.try_into()?,
														position,
													})
												})
												.try_collect::<alloc::Result<_>>()??,
										})
									}
									Fields::Unnamed(args) => meta::Fields::Unnamed(*args),
									Fields::Empty => meta::Fields::Empty,
								},
								constructor,
								parameters: Hash::EMPTY,
								enum_hash: ty.hash,
							},
							#[cfg(feature = "doc")]
							deprecated: variant.deprecated.try_clone()?,
							#[cfg(feature = "doc")]
							docs: variant.docs.try_clone()?,
						})?;
					}

					meta::Kind::Enum { parameters }
				}
			}
		} else {
			meta::Kind::Type { parameters }
		};

		self.install_meta(ContextMeta {
			hash: ty.hash,
			item: Some(ty.item.try_to_owned()?),
			kind,
			#[cfg(feature = "doc")]
			deprecated: ty.common.deprecated.try_clone()?,
			#[cfg(feature = "doc")]
			docs: ty.common.docs.try_clone()?,
		})?;

		Ok(())
	}

	fn install_trait(&mut self, t: &ModuleTrait) -> Result<(), ContextError> {
		if self.traits.try_insert(t.hash, t.handler.clone())?.is_some() {
			return Err(ContextError::ConflictingTrait {
				item: t.item.try_clone()?,
				hash: t.hash,
			});
		}

		#[cfg(feature = "execution")]
		self.iter.trait_installed(t.hash);

		self.install_meta(ContextMeta {
			hash: t.hash,
			item: Some(t.item.try_clone()?),
			kind: meta::Kind::Trait,
			#[cfg(feature = "doc")]
			deprecated: t.common.deprecated.try_clone()?,
			#[cfg(feature = "doc")]
			docs: t.common.docs.try_clone()?,
		})?;

		for f in &t.functions {
			let signature = meta::Signature::from_context(&f.doc, &f.common)?;

			let kind = meta::Kind::Function {
				associated: Some(f.name.kind.try_clone()?),
				trait_hash: None,
				signature,
				is_test: false,
				is_bench: false,
				parameters: Hash::EMPTY.with_function_parameters(f.name.function_parameters),
				#[cfg(feature = "doc")]
				container: Some(t.hash),
				#[cfg(feature = "doc")]
				parameter_types: f.name.parameter_types.try_clone()?,
			};

			let hash = f
				.name
				.kind
				.hash(t.hash)
				.with_function_parameters(f.name.function_parameters);

			let item = if let meta::AssociatedKind::Instance(name) = &f.name.kind {
				let item = t.item.extended(name.as_ref())?;
				let hash = Hash::type_hash(&item);
				Some((hash, item))
			} else {
				None
			};

			self.install_meta(ContextMeta {
				hash,
				item: item.map(|(_, item)| item),
				kind,
				#[cfg(feature = "doc")]
				deprecated: f.common.deprecated.try_clone()?,
				#[cfg(feature = "doc")]
				docs: f.common.docs.try_clone()?,
			})?;
		}

		Ok(())
	}

	fn install_trait_impl(&mut self, i: &ModuleTraitImpl) -> Result<(), ContextError> {
		if !self.types.contains_key(&i.hash) {
			return Err(ContextError::MissingType {
				item: i.item.try_to_owned()?,
				type_info: i.type_info.try_clone()?,
			});
		};

		let Some(handler) = self.traits.get(&i.trait_hash).cloned() else {
			return Err(ContextError::MissingTrait {
				item: i.trait_item.try_clone()?,
				hash: i.hash,
				impl_item: i.item.try_to_owned()?,
				impl_hash: i.hash,
			});
		};

		if let Some(handler) = handler {
			handler.call(&mut TraitContext {
				cx: self,
				item: &i.item,
				hash: i.hash,
				type_info: &i.type_info,
				trait_item: &i.trait_item,
				trait_hash: i.trait_hash,
			})?;
		}

		#[cfg(feature = "doc")]
		self.implemented_traits
			.entry(i.hash)
			.or_try_default()?
			.try_push(i.trait_hash)?;

		Ok(())
	}

	fn install_reexport(&mut self, r: &ModuleReexport) -> Result<(), ContextError> {
		self.install_meta(ContextMeta {
			hash: r.hash,
			item: Some(r.item.try_clone()?),
			kind: meta::Kind::Alias(meta::Alias {
				to: r.to.try_clone()?,
			}),
			#[cfg(feature = "doc")]
			deprecated: None,
			#[cfg(feature = "doc")]
			docs: Docs::EMPTY,
		})?;

		Ok(())
	}

	/// Install a constant constructor.
	fn install_construct(
		&mut self,
		hash: Hash,
		type_info: &AnyTypeInfo,
		construct: &ConstConstructImpl,
	) -> Result<(), ContextError> {
		let old = self.construct.try_insert(hash, construct.clone())?;

		if old.is_some() {
			return Err(ContextError::ConflictingConstConstruct {
				type_info: TypeInfo::from(*type_info),
				hash,
			});
		}

		Ok(())
	}

	fn install_type_info(&mut self, ty: ContextType) -> Result<(), ContextError> {
		let item_hash = Hash::type_hash(&ty.item).with_type_parameters(ty.type_parameters);

		if ty.hash != item_hash {
			return Err(ContextError::TypeHashMismatch {
				type_info: ty.type_info,
				item: ty.item,
				hash: ty.hash,
				item_hash,
			});
		}

		self.constants.try_insert(
			Hash::associated_function(ty.hash, &Protocol::INTO_TYPE_NAME),
			ConstValueBuf::try_from(ty.item.try_to_string()?)?,
		)?;

		if let Some(old) = self.types.try_insert(ty.hash, ty)? {
			return Err(ContextError::ConflictingType {
				item: old.item,
				type_info: old.type_info,
				hash: old.hash,
			});
		}

		Ok(())
	}

	/// Install a function and check for duplicates.
	fn install_item(&mut self, m: &ModuleItem) -> Result<(), ContextError> {
		self.names.insert(&m.item)?;

		let kind = match &m.kind {
			rune::module::ModuleItemKind::Constant(value) => {
				self.constants.try_insert(m.hash, value.try_clone()?)?;
				meta::Kind::Const
			}
			rune::module::ModuleItemKind::Function(f) => {
				self.constants.try_insert(
					Hash::associated_function(m.hash, &Protocol::INTO_TYPE_NAME),
					ConstValueBuf::try_from(m.item.try_to_string()?)?,
				)?;

				let signature = meta::Signature::from_context(&f.doc, &m.common)?;

				self.insert_native_fn(
					&m.item,
					m.hash,
					&f.handler,
					m.common.deprecated.as_deref(),
					#[cfg(feature = "execution")]
					false,
				)?;

				meta::Kind::Function {
					associated: None,
					trait_hash: f.trait_hash,
					signature,
					is_test: false,
					is_bench: false,
					parameters: Hash::EMPTY,
					#[cfg(feature = "doc")]
					container: None,
					#[cfg(feature = "doc")]
					parameter_types: Vec::new(),
				}
			}
			rune::module::ModuleItemKind::Macro(macro_) => {
				// Execution core: certification implies that the handler stored at this hash is
				// the marked std builtin. A non-builtin macro drops it before replacing the
				// handler, and the builtin is certified only after it is stored. The reverse
				// is not guaranteed: after an allocation failure a stored builtin may stay
				// uncertified, which only refuses (fail closed).
				#[cfg(feature = "execution")]
				if !macro_.builtin_macro {
					self.builtin_macros.remove(&m.hash);
				}
				self.macros.try_insert(m.hash, macro_.handler.clone())?;
				#[cfg(feature = "execution")]
				if macro_.builtin_macro {
					self.builtin_macros.try_insert(m.hash)?;
				}
				meta::Kind::Macro
			}
			rune::module::ModuleItemKind::AttributeMacro(macro_) => {
				self.attribute_macros
					.try_insert(m.hash, macro_.handler.clone())?;
				meta::Kind::AttributeMacro
			}
		};

		self.install_meta(ContextMeta {
			hash: m.hash,
			item: Some(m.item.try_to_owned()?),
			kind,
			#[cfg(feature = "doc")]
			deprecated: m.common.deprecated.try_clone()?,
			#[cfg(feature = "doc")]
			docs: m.common.docs.try_clone()?,
		})?;

		Ok(())
	}

	fn install_associated(&mut self, assoc: &ModuleAssociated) -> Result<(), ContextError> {
		let base = assoc.name.kind.hash(assoc.container);
		let hash = base.with_function_parameters(assoc.name.function_parameters);
		#[cfg(feature = "execution")]
		if matches!(&assoc.name.kind, meta::AssociatedKind::Instance(_))
			&& matches!(&assoc.kind, ModuleAssociatedKind::Function(_))
		{
			self.methods.associated(assoc.container, base);
		}
		#[cfg(feature = "execution")]
		if matches!(&assoc.name.kind, meta::AssociatedKind::Instance(n) if n.as_ref() == "sort_by")
			|| hash == SORT_ASSOCIATED
		{
			self.sort_by = false;
			self.sort_by_closed = true;
		}
		#[cfg(feature = "execution")]
		let pipeline_touch = {
			let name = match &assoc.name.kind {
				meta::AssociatedKind::Instance(n) => Some(n.as_ref()),
				_ => None,
			};
			name.and_then(|n| {
				crate::function::pipeline::Method::from_name(n, assoc.name.function_parameters)
			})
			.and_then(|m| {
				let (plain, trait_hash) = match &assoc.kind {
					ModuleAssociatedKind::Function(f) => (true, f.trait_hash),
					_ => (false, None),
				};
				self.pipeline
					.begin(
						m,
						assoc.container,
						plain,
						trait_hash,
						self.iter.builtin_trait(),
					)
					.map(|bit| (m, bit))
			})
		};
		#[cfg(feature = "execution")]
		let plain = || match &assoc.kind {
			ModuleAssociatedKind::Function(f) => {
				(assoc.name.function_parameters == Hash::EMPTY, f.trait_hash)
			}
			ModuleAssociatedKind::Constant(..) => (false, None),
		};
		#[cfg(feature = "execution")]
		let variant_touch = match &assoc.name.kind {
			meta::AssociatedKind::Instance(name) => {
				crate::function::variant::Method::from_name(name).and_then(|m| {
					let (plain, trait_hash) = plain();
					self.variant
						.begin(m, assoc.container, plain, trait_hash)
						.map(|bit| (m, bit))
				})
			}
			_ => None,
		};
		#[cfg(feature = "execution")]
		let object_touch = match &assoc.name.kind {
			meta::AssociatedKind::Instance(name) => {
				crate::function::object::Method::from_name(name).and_then(|m| {
					let (plain, trait_hash) = plain();
					self.object
						.begin(m, assoc.container, plain, trait_hash)
						.map(|bit| (m, bit))
				})
			}
			_ => None,
		};
		#[cfg(feature = "execution")]
		let (text_touch, text_collect) = {
			let instance = match &assoc.name.kind {
				meta::AssociatedKind::Instance(n) => Some(n.as_ref()),
				_ => None,
			};
			let (m, collect) = match instance.map(str::as_bytes) {
				Some(b"collect") => (None, true),
				Some(_) => (
					instance.and_then(crate::function::text::Method::from_name),
					false,
				),
				None => (None, false),
			};
			(
				m.map(|m| {
					let plain = assoc.name.function_parameters == Hash::EMPTY
						&& matches!(&assoc.kind,ModuleAssociatedKind::Function(f) if f.trait_hash.is_none());
					(m, self.text.begin(m, assoc.container, plain))
				}),
				collect,
			)
		};
		#[cfg(feature = "execution")]
		let iter_touch = {
			let instance = match &assoc.name.kind {
				meta::AssociatedKind::Instance(name) => Some(name.as_ref()),
				_ => None,
			};
			match crate::function::iter::touched_name(instance, hash, assoc.container) {
				None => None,
				Some(name) => {
					let (plain, trait_hash) = match &assoc.kind {
						ModuleAssociatedKind::Function(f) => {
							(assoc.name.function_parameters == Hash::EMPTY, f.trait_hash)
						}
						ModuleAssociatedKind::Constant(..) => (false, None),
					};
					self.iter
						.begin_associated(name, assoc.container, plain, trait_hash)
				}
			}
		};
		#[cfg(feature = "execution")]
		let numeric = {
			use crate::function::numeric::{Method, Pair};
			let effective = hash;
			let selected = match &assoc.name.kind {
				meta::AssociatedKind::Instance(name) => Method::from_name(name),
				meta::AssociatedKind::Protocol(p) if p.hash == Protocol::CMP.hash => {
					Some(Method::Cmp)
				}
				_ => [Method::Abs, Method::Round, Method::Powf, Method::Cmp]
					.into_iter()
					.find(|m| {
						effective
							== Hash::associated_function(
								assoc.container,
								self.numeric.name_hash(*m),
							)
					}),
			};
			selected.map(|method| {
				let previous = self.numeric.begin(method, assoc.container);
				let pair = Pair::ALL
					.into_iter()
					.find(|p| p.receiver() == assoc.container && p.method() == method);
				let valid = assoc.name.function_parameters == Hash::EMPTY
					&& (matches!(&assoc.name.kind, meta::AssociatedKind::Instance(_))
						|| matches!(&assoc.name.kind, meta::AssociatedKind::Protocol(p) if method == Method::Cmp && p.hash == Protocol::CMP.hash))
					&& matches!(&assoc.kind, ModuleAssociatedKind::Function(_));
				(pair.filter(|_| valid), previous)
			})
		};
		let Some(info) = self.types.get(&assoc.container).try_cloned()? else {
			return Err(ContextError::MissingContainer {
				container: assoc.container_type_info.try_clone()?,
			});
		};

		// If the associated function is a named instance function - register it
		// under the name of the item it corresponds to unless it's a field
		// function.
		//
		// The other alternatives are protocol functions (which are not free)
		// and plain hashes.
		let item = if let meta::AssociatedKind::Instance(name) = &assoc.name.kind {
			let item = info.item.extended(name.as_ref())?;

			let hash = Hash::type_hash(&item)
				.with_type_parameters(info.type_parameters)
				.with_function_parameters(assoc.name.function_parameters);

			Some((hash, item))
		} else {
			None
		};

		let kind = match &assoc.kind {
			ModuleAssociatedKind::Constant(value) => {
				#[cfg(feature = "execution")]
				if hash == Hash::associated_function(assoc.container, &Protocol::INTO_TYPE_NAME) {
					self.pipeline.close_type_names();
				}
				if let Some((hash, ..)) = item {
					self.constants.try_insert(hash, value.try_clone()?)?;
				}

				self.constants.try_insert(hash, value.try_clone()?)?;
				meta::Kind::Const
			}
			ModuleAssociatedKind::Function(f) => {
				let signature = meta::Signature::from_context(&f.doc, &assoc.common)?;

				if let Some((hash, item)) = &item {
					self.constants.try_insert(
						Hash::associated_function(*hash, &Protocol::INTO_TYPE_NAME),
						ConstValueBuf::try_from(item.try_to_string()?)?,
					)?;

					self.insert_native_fn(
						&assoc.container_type_info,
						*hash,
						&f.handler,
						assoc.common.deprecated.as_deref(),
						#[cfg(feature = "execution")]
						(f.format_default == Some(*hash)),
					)?;
				}

				self.insert_native_fn(
					&assoc.container_type_info,
					hash,
					&f.handler,
					assoc.common.deprecated.as_deref(),
					#[cfg(feature = "execution")]
					(f.format_default == Some(hash)),
				)?;

				meta::Kind::Function {
					associated: Some(assoc.name.kind.try_clone()?),
					trait_hash: f.trait_hash,
					signature,
					is_test: false,
					is_bench: false,
					parameters: Hash::EMPTY
						.with_type_parameters(info.type_parameters)
						.with_function_parameters(assoc.name.function_parameters),
					#[cfg(feature = "doc")]
					container: Some(assoc.container),
					#[cfg(feature = "doc")]
					parameter_types: assoc.name.parameter_types.try_clone()?,
				}
			}
		};

		self.install_meta(ContextMeta {
			hash,
			item: item.map(|(_, item)| item),
			kind,
			#[cfg(feature = "doc")]
			deprecated: assoc.common.deprecated.try_clone()?,
			#[cfg(feature = "doc")]
			docs: assoc.common.docs.try_clone()?,
		})?;

		#[cfg(feature = "execution")]
		if let Some((Some(pair), previous)) = numeric {
			self.numeric.installed(pair, previous);
		}
		#[cfg(feature = "execution")]
		if let Some(touch) = iter_touch {
			self.iter.associated_installed(touch);
		}
		#[cfg(feature = "execution")]
		{
			if let Some((m, bit)) = pipeline_touch {
				self.pipeline.installed(m, bit);
			}
			if let Some((m, bit)) = variant_touch {
				self.variant.installed(m, bit);
			}
			if let Some((m, bit)) = object_touch {
				self.object.installed(m, bit);
			}
			if let Some((m, true)) = text_touch {
				self.text.installed(m);
			}
			if text_collect
				&& assoc.name.function_parameters == Hash::parameters([crate::runtime::Vec::HASH])
			{
				let plain = matches!(&assoc.kind,ModuleAssociatedKind::Function(f) if f.trait_hash==Some(crate::hash_in!(crate,::std::iter::Iterator)))
					&& self.iter.builtin_trait();
				self.text.collect_installed(assoc.container, plain);
			}
		}
		Ok(())
	}

	fn insert_native_fn(
		&mut self,
		display: &dyn fmt::Display,
		hash: Hash,
		handler: &FunctionHandler,
		deprecation: Option<&str>,
		#[cfg(feature = "execution")] format_default: bool,
	) -> Result<(), ContextError> {
		#[cfg(feature = "execution")]
		if !self.format.trusted_install {
			self.format.begin(hash, format_default)?;
		}
		#[cfg(feature = "execution")]
		self.numeric.invalidate_hash(hash);
		#[cfg(feature = "execution")]
		let any_touch = self.pipeline.begin_any(hash);
		#[cfg(feature = "execution")]
		if hash == SORT_ASSOCIATED || hash == SORT_ITEM {
			self.sort_by = false;
			self.sort_by_closed = true;
		}
		if self.functions.contains_key(&hash) {
			return Err(ContextError::ConflictingFunction {
				part: display.try_to_string()?.try_into()?,
				hash,
			});
		}

		self.functions.try_insert(hash, handler.clone())?;
		#[cfg(feature = "execution")]
		if let Some(bit) = any_touch {
			self.pipeline.finish_any(bit);
		}

		if let Some(msg) = deprecation {
			self.deprecations.try_insert(hash, msg.try_to_owned()?)?;
		}

		Ok(())
	}

	/// Get a constant value.
	pub(crate) fn get_const_value(&self, hash: Hash) -> Option<&ConstValue> {
		Some(self.constants.get(&hash)?)
	}
}

impl fmt::Debug for Context {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "Context")
	}
}

impl ConstContext for Context {
	#[inline]
	fn get(&self, hash: Hash) -> Option<&ConstConstructImpl> {
		self.construct.get(&hash)
	}
}

#[cfg(feature = "doc")]
fn fields_to_arguments(fields: &Fields) -> alloc::Result<Box<[meta::DocArgument]>> {
	match *fields {
		Fields::Named(fields) => {
			let mut out = Vec::try_with_capacity(fields.len())?;

			for &name in fields {
				out.try_push(meta::DocArgument {
					name: meta::DocName::Name(Box::try_from(name)?),
					base: Hash::EMPTY,
					generics: Box::default(),
				})?;
			}

			Box::try_from(out)
		}
		Fields::Unnamed(args) => {
			let mut out = Vec::try_with_capacity(args)?;

			for n in 0..args {
				out.try_push(meta::DocArgument {
					name: meta::DocName::Index(n),
					base: Hash::EMPTY,
					generics: Box::default(),
				})?;
			}

			Box::try_from(out)
		}
		Fields::Empty => Ok(Box::default()),
	}
}

#[cfg(test)]
static_assertions::assert_impl_all!(Context: Send, Sync);

#[cfg(all(test, feature = "execution"))]
mod numeric_tests;

#[cfg(all(test, feature = "execution"))]
mod c11_sort_certificates {
	use super::*;
	#[test]
	#[cfg_attr(miri, ignore = "default-module Context; native certificate control")]
	fn missing_trusted_conflicting_and_unrelated_sort_names_are_closed() {
		assert_eq!(
			SORT_RECEIVER,
			Hash::type_hash(crate::item!(::std::vec::Vec))
		);
		assert_eq!(
			SORT_ASSOCIATED,
			Hash::associated_function(SORT_RECEIVER, Hash::ident("sort_by"))
		);
		assert_eq!(
			SORT_ITEM,
			Hash::type_hash(crate::item!(::std::vec::Vec::sort_by))
		);
		assert!(!Context::new().sort_by());
		let mut cx = Context::with_default_modules().unwrap();
		assert!(cx.sort_by());
		let mut conflicting = Module::new();
		conflicting
			.associated_function(
				"sort_by",
				|_: &mut crate::runtime::Vec, _: &crate::runtime::Function| {},
			)
			.unwrap();
		assert!(cx.install(conflicting).is_err());
		assert!(!cx.sort_by());
		let mut cx = Context::with_default_modules().unwrap();
		let mut unrelated = Module::new();
		unrelated
			.associated_function("sort_by", |_: i64, _: i64| {})
			.unwrap();
		cx.install(unrelated).unwrap();
		assert!(!cx.sort_by());
	}
}

#[cfg(all(test, feature = "execution"))]
mod c12_iter_certificates {
	use super::*;
	use crate::function::iter::Name;

	#[test]
	#[cfg_attr(miri, ignore = "default-module Context; native certificate control")]
	fn default_modules_certify_and_foreign_names_close() {
		let empty = Context::new();
		for name in Name::ALL {
			assert!(!empty.iter_method(name), "{name:?}");
		}

		let cx = Context::with_default_modules().unwrap();
		for name in Name::ALL {
			assert!(cx.iter_method(name), "{name:?}");
		}

		// A foreign `iter` on a representable receiver that stock leaves without one.
		let mut cx = Context::with_default_modules().unwrap();
		let mut foreign = Module::new();
		foreign.associated_function("iter", |_: i64| 0i64).unwrap();
		cx.install(foreign).unwrap();
		assert!(!cx.iter_method(Name::Iter));
		assert!(cx.iter_method(Name::Next));
		assert!(cx.iter_method(Name::Any));

		// A foreign `any` on Vec, which stock does not give one.
		let mut cx = Context::with_default_modules().unwrap();
		let mut foreign = Module::new();
		foreign
			.associated_function("any", |_: &crate::runtime::Vec, _: i64| false)
			.unwrap();
		cx.install(foreign).unwrap();
		assert!(!cx.iter_method(Name::Any));
		assert!(cx.iter_method(Name::Iter));

		// A conflicting `next` on the slice iterator fails and leaves `next` closed.
		let mut cx = Context::with_default_modules().unwrap();
		let mut conflicting = Module::new();
		conflicting
			.associated_function("next", |_: &mut crate::runtime::slice::Iter| None::<i64>)
			.unwrap();
		assert!(cx.install(conflicting).is_err());
		assert!(!cx.iter_method(Name::Next));

		// A conflicting `iter` on Vec after the builtin: insert-once refuses it, and its
		// pre-insert touch closes `iter` (no per-insert provider scan is involved).
		let mut cx = Context::with_default_modules().unwrap();
		let mut conflicting = Module::new();
		conflicting
			.associated_function("iter", |_: &crate::runtime::Vec| 0i64)
			.unwrap();
		assert!(cx.install(conflicting).is_err());
		assert!(!cx.iter_method(Name::Iter));
		assert!(cx.iter_method(Name::Next));
	}
}

// C13 stock-oracle raw-hash insertion fixture. Test-only; no product API.
#[cfg(all(test, feature = "execution"))]
impl Context {
	pub(crate) fn c13_insert_raw_fixture(
		&mut self,
		hash: Hash,
		handler: FunctionHandler,
	) -> Result<(), ContextError> {
		self.insert_native_fn(
			&"C13 raw fixture",
			hash,
			&handler,
			None,
			#[cfg(feature = "execution")]
			false,
		)
	}
}

#[cfg(all(test, feature = "execution"))]
mod c18b3c_vec_constructor_certificates {
	use super::*;

	#[test]
	fn vec_constructor_receiver_identity_matches_runtime_hash() {
		assert_eq!(
			VEC_CONSTRUCTOR_RECEIVER,
			Hash::type_hash(crate::item!(::std::vec::Vec))
		);
	}
}

#[cfg(all(test, feature = "execution"))]
mod formatter_epoch_tests {
	use super::*;
	fn public_install_is_untrusted(context: &mut Context) {
		assert!(!context.format.trusted_install);
		let mut module = Module::with_crate("after_epoch").unwrap();
		module.host_function("probe", || 7i64).unwrap();
		context.install(module).unwrap();
		assert!(context.format_snapshot().is_some());
	}
	#[test]
	fn public_installs_after_default_construction_do_not_inherit_the_epoch() {
		for stdio in [false, true] {
			let mut context = Context::with_config(stdio).unwrap();
			assert!(context.format_snapshot().is_none());
			public_install_is_untrusted(&mut context);
		}
	}
	#[test]
	fn partial_failure_and_unwind_clear_the_non_reentrant_guard() {
		let mut failed = Context::new();
		let mut fail = || -> core::result::Result<(), ()> {
			let mut epoch = StdFormatEpoch::enter(&mut failed);
			epoch
				.install(crate::modules::core::module().unwrap())
				.unwrap();
			epoch
				.install(crate::modules::char::module().unwrap())
				.unwrap();
			Err(())
		};
		let result = fail();
		assert!(result.is_err());
		public_install_is_untrusted(&mut failed);
		let mut panicked = Context::new();
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let mut epoch = StdFormatEpoch::enter(&mut panicked);
			epoch
				.install(crate::modules::core::module().unwrap())
				.unwrap();
			epoch
				.install(crate::modules::char::module().unwrap())
				.unwrap();
			panic!("epoch unwind control");
		}));
		assert!(result.is_err());
		public_install_is_untrusted(&mut panicked);
		let mut context = Context::new();
		{
			let mut epoch = StdFormatEpoch::enter(&mut context);
			let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				let _nested = StdFormatEpoch::enter(&mut epoch);
			}));
			assert!(result.is_err());
			assert!(epoch.format.trusted_install);
		}
		public_install_is_untrusted(&mut context);
	}
	#[test]
	fn appended_functions_on_returned_std_modules_never_inherit_default_provenance() {
		let mut module = crate::modules::char::module().unwrap();
		module.host_function("external", || 7i64).unwrap();
		let mut context = Context::with_default_modules().unwrap();
		assert!(context.format_snapshot().is_none());
		// Use the production item-install path for the new appended function,
		// without replaying std type declarations already installed above.
		context.install_item(module.items.last().unwrap()).unwrap();
		assert!(context.format_snapshot().is_some());
		assert!(!context.format.trusted_install);
	}

	#[test]
	fn constructor_calls_only_the_reviewed_closed_std_factory_list() {
		let source = include_str!("context.rs");
		let begin = source.find("pub fn with_config(").unwrap();
		let end = source[begin..]
			.find("pub fn with_default_modules(")
			.unwrap() + begin;
		let actual: std::vec::Vec<_> = source[begin..end]
			.lines()
			.filter_map(|line| {
				line.trim()
					.strip_prefix("this.install(")
					.map(|v| v.strip_suffix(")?;").unwrap())
			})
			.collect();
		let expected = [
			"crate::modules::iter::module()?",
			"crate::modules::core::module()?",
			"crate::modules::cmp::module()?",
			"crate::modules::any::module()?",
			"crate::modules::clone::module()?",
			"crate::modules::num::module()?",
			"crate::modules::hash::module()?",
			"crate::modules::string::module()?",
			"crate::modules::bytes::module()?",
			"crate::modules::collections::module()?",
			"crate::modules::collections::hash_map::module()?",
			"crate::modules::collections::hash_set::module()?",
			"crate::modules::collections::vec_deque::module()?",
			"crate::modules::char::module()?",
			"crate::modules::f64::module()?",
			"crate::modules::f64::consts::module()?",
			"crate::modules::tuple::module()?",
			"crate::modules::fmt::module()?",
			"crate::modules::future::module()?",
			"crate::modules::i64::module()?",
			"crate::modules::u64::module()?",
			"crate::modules::io::module(stdio)?",
			"crate::modules::macros::module()?",
			"crate::modules::macros::builtin::module()?",
			"crate::modules::mem::module()?",
			"crate::modules::object::module()?",
			"crate::modules::ops::module()?",
			"crate::modules::ops::generator::module()?",
			"crate::modules::option::module()?",
			"crate::modules::result::module()?",
			"crate::modules::stream::module()?",
			"crate::modules::test::module()?",
			"crate::modules::vec::module()?",
			"crate::modules::slice::module()?",
		];
		assert_eq!(actual, expected);
		assert!(expected
			.iter()
			.all(|factory| factory.starts_with("crate::modules::")
				&& (factory.ends_with("module()?") || factory.ends_with("module(stdio)?"))));
	}
}
