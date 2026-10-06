//! Base-only contracts for the 0179 registration change.
use std::cell::RefCell;
use std::format;
use std::string::String as StdString;
use std::vec::Vec as StdVec;

use super::{ConstValueBuf, Context};
use crate::alloc::prelude::*;
use crate::item::ComponentRef;
use crate::runtime::Protocol;
use crate::{Any, Hash, ItemBuf, Module};

std::thread_local! {
    static EVENTS: RefCell<Option<StdVec<StdString>>> = const { RefCell::new(None) };
}

pub(super) fn record(stage: &'static str, hash: Hash) {
    EVENTS.with(|events| {
        if let Some(events) = events.borrow_mut().as_mut() {
            events.push(format!("{stage} {hash:?}"));
        }
    });
}

pub(super) fn display_fixtures() -> StdVec<(ItemBuf, StdString)> {
    use ComponentRef::{Crate, Id, Str};
    let mut rows = StdVec::new();
    for (parts, text) in [
        (std::vec![], "{root}"),
        (std::vec![Str("one")], "one"),
        (std::vec![Str("one"), Str("two")], "one::two"),
        (
            std::vec![Crate("std"), Str("vec"), Str("Vec"), Str("len")],
            "::std::vec::Vec::len",
        ),
        (std::vec![Crate("only")], "::only"),
        (std::vec![Str("a"), Crate("b")], "a::::b"),
        (std::vec![Crate("a"), Crate("b"), Str("c")], "::a::::b::c"),
        (std::vec![Str("")], ""),
        (std::vec![Str(""), Str("")], "::"),
        (std::vec![Crate("")], "::"),
        (std::vec![Str(""), Crate(""), Str("")], "::::::"),
        (std::vec![Str("μ"), Str("日本"), Str("🦦")], "μ::日本::🦦"),
        (std::vec![Id(7)], "$7"),
        (std::vec![Id(7), Str("x")], "$7::x"),
        (std::vec![Str("x"), Id(7)], "x::$7"),
        (std::vec![Str("x"), Id(7), Str("y")], "x::$7::y"),
        (std::vec![Crate("x"), Id(7), Str("y")], "::x::$7::y"),
    ] {
        rows.push((ItemBuf::with_item(parts).unwrap(), StdString::from(text)));
    }
    let long = "x".repeat(8192);
    rows.push((ItemBuf::with_item([Str(&long)]).unwrap(), long));
    let mut deep = ItemBuf::new();
    for _ in 0..1024 {
        deep.push("x").unwrap();
    }
    rows.push((deep, std::vec!["x"; 1024].join("::")));
    rows
}

#[test]
fn item_display_bytes_are_frozen_before_the_candidate() {
    for (item, expected) in display_fixtures() {
        assert_eq!(item.try_to_string().unwrap().as_str(), expected);
    }
}

fn trace(stdio: bool) -> StdString {
    EVENTS.with(|events| {
        assert!(events.borrow().is_none());
        *events.borrow_mut() = Some(StdVec::new());
    });
    let context = Context::with_config(stdio).unwrap();
    let events = EVENTS.with(|events| events.borrow_mut().take().unwrap());
    drop(context);
    events.join("\n") + "\n"
}

#[test]
fn registration_order_matches_the_base() {
    let state = format!(
        "# stdio=true\n{}# stdio=false\n{}",
        trace(true),
        trace(false)
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(feature = "doc") {
        "src/compile/context_startup_order.golden"
    } else {
        "src/compile/context_startup_order_nodoc.golden"
    });
    if std::env::var_os("RUNE_UPDATE_STARTUP_GOLDEN").is_some() {
        std::fs::write(&path, &state).unwrap();
    } else {
        let frozen = std::fs::read_to_string(path).unwrap();
        assert_eq!(state, frozen);
        let mut corrupted: StdVec<_> = state.lines().collect();
        corrupted.swap(3, 4);
        assert_ne!(corrupted.join("\n") + "\n", frozen);
    }
}

#[derive(Any)]
#[rune(crate, item = ::startup)]
struct Container;

#[test]
fn custom_associated_constant_values_and_errors_are_unchanged() {
    for stdio in [true, false] {
        let mut context = Context::with_config(stdio).unwrap();
        let mut module = Module::with_crate("startup").unwrap();
        module.ty::<Container>().unwrap();
        module
            .associated_function("méthod", |_: &Container| 42i64)
            .unwrap();
        context.install(&module).unwrap();
        let item = ItemBuf::with_crate_item("startup", ["Container", "méthod"]).unwrap();
        let hash = Hash::associated_function(Hash::type_hash(&item), &Protocol::INTO_TYPE_NAME);
        let expected = ConstValueBuf::try_from(
            crate::alloc::String::try_from("::startup::Container::méthod").unwrap(),
        )
        .unwrap();
        assert_eq!(
            format!("{:?}", context.constants.get(&hash).unwrap()),
            format!("{expected:?}")
        );
        assert!(context.install(&module).is_err());
        let other = Context::with_config(stdio).unwrap();
        assert!(other.lookup_function(Hash::type_hash(&item)).is_none());
        assert!(context.lookup_function(Hash::type_hash(&item)).is_some());
    }
}

#[test]
fn every_module_first_use_completes_with_and_without_stdio() {
    for stdio in [true, false] {
        let source = if stdio {
            include_str!("startup_fixtures/first-use-stdio.rn")
        } else {
            include_str!("startup_fixtures/first-use.rn")
        };
        let context = Context::with_config(stdio).unwrap();
        let mut sources = crate::Sources::new();
        sources
            .insert(crate::Source::new("main", source).unwrap())
            .unwrap();
        let mut diagnostics = crate::Diagnostics::new();
        let unit = crate::prepare(&mut sources)
            .with_context(&context)
            .with_diagnostics(&mut diagnostics)
            .with_options(&crate::Options::default())
            .build()
            .unwrap();
        let mut vm = crate::Vm::new(
            crate::sync::Arc::try_new(context.runtime().unwrap()).unwrap(),
            crate::sync::Arc::try_new(unit).unwrap(),
        );
        let value =
            futures_executor::block_on(vm.execute(["main"], ((),)).unwrap().async_complete())
                .unwrap_or_else(|error| panic!("first-use failed: {error}"));
        assert_eq!(crate::from_value::<i64>(value).unwrap(), 42);
    }
}
