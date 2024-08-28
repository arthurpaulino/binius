// Copyright 2024 Ulvetanna Inc.
use crate::polynomial::{CompositionPoly, Error};
use binius_field::{ExtensionField, PackedField, TowerField};
use binius_utils::thread_local_mut::ThreadLocalMut;
use std::sync::Arc;

/// Corresponds to the index of another expression within the same Vec/other type of contiguous memory.
pub type ExprId = usize;

/// Describes computation symbolically. This is used internally by ArithCircuitPoly.
///
/// ExprIds used by an Expr has to be less than the index of the Expr itself within the ArithCircuitPoly,
/// to ensure it represents a directed acyclic graph that can be computed in sequence.
#[derive(Debug)]
pub enum Expr<F: TowerField> {
	Const(F),
	Var(usize),
	Add(ExprId, ExprId),
	Mul(ExprId, ExprId),
	Pow(ExprId, u64),
}

/// Describes polynomial evaluations using a directed acyclic graph of expressions.
///
/// This is meant as an alternative to a hard-coded CompositionPoly.
///
/// The advantage over a hard coded CompositionPoly is that this can be constructed and manipulated dynamically at runtime.
#[derive(Debug)]
pub struct ArithCircuitPoly<F: TowerField, P: PackedField<Scalar: ExtensionField<F>>> {
	/// The last expression is the "top level expression" which depends on previous entries
	exprs: Arc<[Expr<F>]>,
	/// This is used internally to avoid allocations every time an evaluation happens
	evals: ThreadLocalMut<Box<[P]>>,
	degree: usize,
	n_vars: usize,
}

impl<F: TowerField, P: PackedField<Scalar: ExtensionField<F>>> ArithCircuitPoly<F, P> {
	pub fn new(exprs: Vec<Expr<F>>) -> Self {
		let degree = {
			let mut degrees = vec![0; exprs.len()];
			for (i, expr) in exprs.iter().enumerate() {
				degrees[i] = match expr {
					Expr::Const(_) => 0,
					Expr::Var(_) => 1,
					Expr::Add(x, y) => {
						debug_assert!(*x < i);
						debug_assert!(*y < i);
						std::cmp::max(degrees[*x], degrees[*y])
					}
					Expr::Mul(x, y) => {
						debug_assert!(*x < i);
						debug_assert!(*y < i);
						degrees[*x] + degrees[*y]
					}
					Expr::Pow(x, n) => {
						debug_assert!(*x < i);
						degrees[*x] * (*n as usize)
					}
				}
			}
			*degrees.last().unwrap()
		};
		let n_vars = exprs
			.iter()
			.map(|x| {
				if let Expr::Var(index) = x {
					index + 1
				} else {
					0
				}
			})
			.max()
			.unwrap_or(0);
		let exprs = exprs.into();
		Self {
			exprs,
			degree,
			n_vars,
			evals: Default::default(),
		}
	}
}

impl<F: TowerField, P: PackedField<Scalar: ExtensionField<F>>> CompositionPoly<P>
	for ArithCircuitPoly<F, P>
{
	fn evaluate(&self, query: &[P]) -> Result<P, Error> {
		if query.len() != self.n_vars {
			return Err(Error::IncorrectQuerySize {
				expected: self.n_vars,
			});
		}
		let result = self.evals.with_mut(
			|| vec![P::zero(); self.exprs.len()].into_boxed_slice(),
			|evals| unsafe {
				for (i, expr) in self.exprs.iter().enumerate() {
					evals[i] = match expr {
						Expr::Const(value) => P::broadcast((*value).into()),
						Expr::Var(index) => *query.get_unchecked(*index),
						Expr::Add(x, y) => *evals.get_unchecked(*x) + *evals.get_unchecked(*y),
						Expr::Mul(x, y) => *evals.get_unchecked(*x) * *evals.get_unchecked(*y),
						Expr::Pow(id, exp) => pow(*evals.get_unchecked(*id), *exp),
					}
				}
				*evals.last().unwrap()
			},
		);
		Ok(result)
	}

	fn degree(&self) -> usize {
		self.degree
	}

	fn n_vars(&self) -> usize {
		self.n_vars
	}

	fn binary_tower_level(&self) -> usize {
		F::TOWER_LEVEL
	}
}

fn pow<P: PackedField>(value: P, exp: u64) -> P {
	let mut res = P::one();
	for i in (0..64).rev() {
		res = res.square();
		if ((exp >> i) & 1) == 1 {
			res.mul_assign(value)
		}
	}
	res
}

#[cfg(test)]
mod tests {
	use super::{ArithCircuitPoly, Expr};
	use crate::polynomial::{test_utils::macros::felts, CompositionPoly};
	use binius_field::{
		BinaryField128b, BinaryField16b, BinaryField1b, BinaryField8b, ExtensionField,
		PackedBinaryField128x1b, PackedBinaryField1x128b, PackedBinaryField8x16b, PackedField,
		TowerField,
	};

	#[test]
	fn test_const() {
		fn assert_valid_const_circuit<F: TowerField, P: PackedField<Scalar: ExtensionField<F>>>(
			value: F,
		) {
			let circuit: ArithCircuitPoly<F, P> = ArithCircuitPoly::new(vec![Expr::Const(value)]);
			assert_eq!(circuit.binary_tower_level(), F::TOWER_LEVEL);
			assert_eq!(circuit.degree(), 0);
			assert_eq!(circuit.n_vars(), 0);
			assert_eq!(circuit.evaluate(&[]).unwrap(), P::broadcast(value.into()));
		}

		assert_valid_const_circuit::<BinaryField1b, PackedBinaryField128x1b>(BinaryField1b::one());
		assert_valid_const_circuit::<BinaryField1b, PackedBinaryField8x16b>(BinaryField1b::one());
		assert_valid_const_circuit::<BinaryField8b, PackedBinaryField8x16b>(BinaryField8b::new(13));
		assert_valid_const_circuit::<BinaryField128b, PackedBinaryField1x128b>(
			BinaryField128b::new(0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF),
		);
	}

	#[test]
	fn test_var() {
		type F = BinaryField8b;
		type P = PackedBinaryField8x16b;
		let circuit: ArithCircuitPoly<F, P> = ArithCircuitPoly::new(vec![Expr::Var(0)]);
		assert_eq!(circuit.binary_tower_level(), F::TOWER_LEVEL);
		assert_eq!(circuit.degree(), 1);
		assert_eq!(circuit.n_vars(), 1);
		assert_eq!(
			circuit
				.evaluate(&[P::from_scalars(
					felts!(BinaryField16b[0, 1, 2, 3, 122, 123, 124, 125])
				)])
				.unwrap(),
			P::from_scalars(felts!(BinaryField16b[0, 1, 2, 3, 122, 123, 124, 125]))
		);
	}

	#[test]
	fn test_add() {
		type F = BinaryField8b;
		type P = PackedBinaryField8x16b;
		let circuit: ArithCircuitPoly<F, P> =
			ArithCircuitPoly::new(vec![Expr::Const(F::new(123)), Expr::Var(0), Expr::Add(0, 1)]);
		assert_eq!(circuit.binary_tower_level(), F::TOWER_LEVEL);
		assert_eq!(circuit.degree(), 1);
		assert_eq!(circuit.n_vars(), 1);
		assert_eq!(
			circuit
				.evaluate(&[P::from_scalars(
					felts!(BinaryField16b[0, 1, 2, 3, 122, 123, 124, 125])
				)])
				.unwrap(),
			P::from_scalars(felts!(BinaryField16b[123, 122, 121, 120, 1, 0, 7, 6]))
		);
	}

	#[test]
	fn test_mul() {
		type F = BinaryField8b;
		type P = PackedBinaryField8x16b;
		let circuit: ArithCircuitPoly<F, P> =
			ArithCircuitPoly::new(vec![Expr::Const(F::new(123)), Expr::Var(0), Expr::Mul(0, 1)]);
		assert_eq!(circuit.binary_tower_level(), F::TOWER_LEVEL);
		assert_eq!(circuit.degree(), 1);
		assert_eq!(circuit.n_vars(), 1);
		assert_eq!(
			circuit
				.evaluate(&[P::from_scalars(
					felts!(BinaryField16b[0, 1, 2, 3, 122, 123, 124, 125])
				)])
				.unwrap(),
			P::from_scalars(felts!(BinaryField16b[0, 123, 157, 230, 85, 46, 154, 225]))
		);
	}

	#[test]
	fn test_pow() {
		type F = BinaryField8b;
		type P = PackedBinaryField8x16b;
		let circuit: ArithCircuitPoly<F, P> =
			ArithCircuitPoly::new(vec![Expr::Var(0), Expr::Pow(0, 13)]);
		assert_eq!(circuit.binary_tower_level(), F::TOWER_LEVEL);
		assert_eq!(circuit.degree(), 13);
		assert_eq!(circuit.n_vars(), 1);
		assert_eq!(
			circuit
				.evaluate(&[P::from_scalars(
					felts!(BinaryField16b[0, 1, 2, 3, 122, 123, 124, 125])
				)])
				.unwrap(),
			P::from_scalars(felts!(BinaryField16b[0, 1, 2, 3, 200, 52, 51, 115]))
		);
	}

	#[test]
	fn test_mixed() {
		type F = BinaryField8b;
		type P = PackedBinaryField8x16b;
		let circuit: ArithCircuitPoly<F, P> = ArithCircuitPoly::new(vec![
			Expr::Var(0),
			Expr::Var(1),
			Expr::Const(F::new(123)),
			Expr::Pow(0, 2),
			Expr::Add(1, 2),
			Expr::Mul(3, 4),
		]);

		assert_eq!(circuit.binary_tower_level(), F::TOWER_LEVEL);
		assert_eq!(circuit.degree(), 3);
		assert_eq!(circuit.n_vars(), 2);
		assert_eq!(
			circuit
				.evaluate(&[
					P::from_scalars(felts!(BinaryField16b[0, 1, 2, 3, 4, 5, 6, 7])),
					P::from_scalars(felts!(BinaryField16b[100, 101, 102, 103, 104, 105, 106, 107]))
				])
				.unwrap(),
			P::from_scalars(felts!(BinaryField16b[0, 30, 59, 36, 151, 140, 170, 176]))
		);
	}
}
