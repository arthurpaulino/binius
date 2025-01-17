use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use binius_circuits::builder::{witness::Builder, ConstraintSystemBuilder};
use binius_core::{
	constraint_system::{
		channel::{Boundary, ChannelId, FlushDirection},
		validate::validate_witness,
	},
	oracle::{MultilinearOracleSet, OracleId, ShiftVariant},
	witness::MultilinearExtensionIndex,
};
use binius_field::{
	arch::OptimalUnderlier, as_packed_field::PackScalar, BinaryField128b, BinaryField32b,
	ExtensionField, TowerField,
};
// use binius_macros::arith_expr;
use binius_utils::checked_arithmetics::log2_ceil_usize;
use bumpalo::Bump;
use bytemuck::Pod;

struct Oracles {
	n: OracleId,
	s: OracleId,
	n_next: OracleId,
	s_next: OracleId,
}

/// n   s | n_next s_next
/// 5, 15 |      4     10
/// 4, 10 |      3      6
/// 3,  6 |      2      3
/// 2,  3 |      1      1
/// 1,  1 |      0      0
fn constrain<U, F>(
	builder: &mut ConstraintSystemBuilder<U, F>,
	count: usize,
) -> Result<(Oracles, ChannelId)>
where
	U: PackScalar<F>,
	F: TowerField,
{
	let log_size = log2_ceil_usize(count);
	let n = builder.add_committed("n", log_size, BinaryField32b::TOWER_LEVEL);
	let s = builder.add_committed("s", log_size, BinaryField32b::TOWER_LEVEL);
	let n_next = builder.add_shifted("n_next", n, 1, log_size, ShiftVariant::LogicalRight)?;
	let s_next = builder.add_shifted("s_next", s, 1, log_size, ShiftVariant::LogicalRight)?;

	// builder.assert_zero(
	// 	"n - n_next - 1 = 0",
	// 	[n, n_next],
	// 	arith_expr!(BinaryField32b[n, n_next] = n - n_next - 1).convert_field(),
	// );
	// builder.assert_zero(
	// 	"n + s_next - s = 0",
	// 	[n, s_next, s],
	// 	arith_expr!(BinaryField32b[n, s_next, s] = n + s_next - s).convert_field(),
	// );

	builder.assert_not_zero(n); // Avoid underflowing `n_next`

	let channel = builder.add_channel();
	builder.send(channel, count, [n, s]);
	builder.receive(channel, count, [n_next, s_next]);
	Ok((
		Oracles {
			n,
			s,
			n_next,
			s_next,
		},
		channel,
	))
}

fn synthesize<
	'a,
	F: TowerField + ExtensionField<BinaryField32b>,
	U: PackScalar<F> + PackScalar<BinaryField32b> + Pod,
>(
	allocator: &'a Bump,
	oracles_set: MultilinearOracleSet<F>,
	oracles: Oracles,
	ns: &[u32],
	ss: &[u32],
) -> Result<MultilinearExtensionIndex<'a, U, F>> {
	assert_eq!(ns.len(), ss.len());
	let witness = Builder::new(allocator, Rc::new(RefCell::new(oracles_set)));
	let Oracles {
		n,
		s,
		n_next,
		s_next,
	} = oracles;
	witness
		.new_column::<BinaryField32b>(n)
		.as_mut_slice()
		.copy_from_slice(&ns[..ns.len() - 1]);
	witness
		.new_column::<BinaryField32b>(s)
		.as_mut_slice()
		.copy_from_slice(&ss[..ss.len() - 1]);
	witness
		.new_column::<BinaryField32b>(n_next)
		.as_mut_slice()
		.copy_from_slice(&ns[1..]);
	witness
		.new_column::<BinaryField32b>(s_next)
		.as_mut_slice()
		.copy_from_slice(&ss[1..]);
	witness.build()
}

fn main() -> Result<()> {
	let n: u32 = 8;

	let mut builder = ConstraintSystemBuilder::<OptimalUnderlier, BinaryField128b>::new();
	let (oracles, channel_id) = constrain(&mut builder, n as usize)?;

	let constraint_system = builder.build()?;

	let ns: Vec<u32> = (0..=n).rev().collect();
	let ss: Vec<u32> = ns.iter().map(|n| n * (n + 1) / 2).collect();
	let allocator = Bump::new();
	let witness: MultilinearExtensionIndex<'_, OptimalUnderlier, _> =
		synthesize(&allocator, constraint_system.oracles.clone(), oracles, &ns, &ss)?;

	let f = |x| BinaryField32b::new(x).into();

	let boundaries = [
		Boundary {
			values: vec![f(n), f(ss[0])],
			channel_id,
			direction: FlushDirection::Pull,
			multiplicity: 1,
		},
		Boundary {
			values: vec![f(0), f(0)],
			channel_id,
			direction: FlushDirection::Push,
			multiplicity: 1,
		},
	];

	validate_witness(&constraint_system, &boundaries, &witness)?;
	Ok(())
}
