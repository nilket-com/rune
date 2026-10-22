//! A nominal from any compile must preserve the receiver-domain proof.
use super::*;
use crate::experimental_core::receiver_domain::UNREPRESENTED;

fn rejected(result: Result<Nominal>) {
	assert_eq!(result.unwrap_err().refusal, Some(Refusal::TypeExpr));
}

#[test]
fn excluded_nominals_refuse_every_shape_and_enum_parent_before_publication() {
	for receiver in UNREPRESENTED {
		let hash = receiver.into_inner();
		let mut staging = Staging::default();
		// Plain staging also covers enum and variant type-value identities.
		rejected(staging.stage(hash, format_args!("Excluded")));
		for shape in [Shape::Named, Shape::Empty, Shape::Tuple(2)] {
			rejected(staging.stage_shape(hash, format_args!("Excluded"), shape));
			rejected(staging.stage_variant(hash, format_args!("E::V"), shape, 0xc18b3b01));
			rejected(staging.stage_variant(0xc18b3b02, format_args!("E::V"), shape, hash));
			rejected(staging.stage_variant_order(
				0xc18b3b03,
				format_args!("E::V"),
				shape,
				hash,
				2,
				&["x"],
			));
		}
		assert_eq!(staging.len(), 0);
		assert!(staging.names.is_empty());
		assert!(lookup(hash).unwrap().is_none());
		rejected(intern(hash, "Excluded"));
		rejected(intern_fmt(hash, format_args!("Excluded")));
		assert!(lookup(hash).unwrap().is_none());
	}
}

#[test]
fn nominal_publication_rechecks_the_domain_before_any_batch_write() {
	for receiver in UNREPRESENTED {
		for parent in [false, true] {
			let mut staging = Staging::default();
			staging.stage(0xc18b3b10, format_args!("Good")).unwrap();
			staging.stage(0xc18b3b11, format_args!("Altered")).unwrap();
			// Exercise the publication backstop independently of staging.
			if parent {
				staging.entries[1].enum_hash = receiver.into_inner();
			} else {
				staging.entries[1].hash = receiver.into_inner();
			}
			let mut table = Interner::with_limits(Limits::default()).unwrap();
			let before = table.reserved_bytes();
			let mut remap = staging.prepare_remap().unwrap();
			assert_eq!(
				table.commit(&staging, &mut remap).unwrap_err().refusal,
				Some(Refusal::TypeExpr)
			);
			assert_eq!(table.len(), 0);
			assert!(table.names.is_empty());
			assert_eq!(table.reserved_bytes(), before);
		}
	}
}

#[test]
fn existing_nominal_lookup_does_not_bypass_the_domain_check() {
	for receiver in UNREPRESENTED {
		let mut table = Interner::with_limits(Limits::default()).unwrap();
		// An isolated corrupted table cannot make a reserved identity admissible.
		// Never seed the process-global table with this descriptor.
		table
			.append(receiver.into_inner(), "Excluded", Shape::Named, 0, 0, 8)
			.unwrap();
		rejected(table.intern(receiver.into_inner(), "Existing"));
		rejected(table.intern_fmt(receiver.into_inner(), format_args!("Existing")));
	}
}
