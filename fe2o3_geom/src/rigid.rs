//! Generic (combinatorial) rigidity of a 2-D bar-joint framework.
//!
//! A bar-joint framework is a set of joints in the plane connected by rigid bars. Its
//! *generic* rigidity -- whether it can flex when the joints sit in general position --
//! depends only on the underlying graph, not on the exact coordinates. Laman's theorem
//! makes this precise: a graph on `n` joints is minimally rigid in the plane exactly when
//! it has `2n - 3` bars and every `k`-joint subset spans at most `2k - 3` bars. The bars
//! that are independent in this sense form the *2-D rigidity matroid*, and its rank is what
//! decides rigidity.
//!
//! This module computes that rank with the `(2, 3)`-pebble game (Lee and Streinu, "Pebble
//! game algorithms and sparse graphs"), which tests each bar for independence in turn and
//! runs in low polynomial time without ever forming the rigidity matrix. From the rank it
//! reports the internal degrees of freedom and the count of redundant bars.
//!
//! The joints are taken as [`Pt`] so a caller passes its geometry directly, but the generic
//! result reads only the joint *count* and the bar list: two frameworks with the same graph
//! share a verdict whatever their coordinates. Special positions (three collinear joints,
//! say) can lose rank in reality, and this generic test deliberately does not model that --
//! it answers the question a truss puzzle asks, where any two valid layouts are equivalent.

use crate::planar::Pt;

use oxedyne_fe2o3_core::prelude::*;

// The plane gives each joint two pebbles (its two translational freedoms), and a bar is
// admitted once its endpoints between them hold l + 1 = 4 pebbles, where l = 3 is the count
// of rigid-body motions of the plane (two translations and one rotation).
const PEBBLES_PER_JOINT:	usize = 2;      // k, the spatial dimension
const ADMIT_THRESHOLD:		usize = 4;      // l + 1

/// The rigidity verdict for a 2-D bar-joint framework.
///
/// `dof` counts the ways the framework can flex beyond the three rigid-body motions of the
/// plane; it is zero exactly when the framework is rigid. `excess` counts bars that add no
/// stiffness -- remove any one and the rank is unchanged; it is zero exactly when the
/// framework carries no redundancy. A framework is minimally rigid when both are zero.
pub struct Rigidity {
    pub rank:	usize,  // independent bars: the rank of the 2-D rigidity matroid
    pub dof:	usize,  // internal degrees of freedom, 2n - 3 - rank
    pub excess:	usize,  // redundant bars, present bars - rank
}

impl Rigidity {
    /// Is the framework rigid (no internal freedom)?
    pub fn is_rigid(&self) -> bool {
        self.dof == 0
    }

    /// Is the framework minimally rigid (rigid and free of redundant bars)?
    pub fn is_minimally_rigid(&self) -> bool {
        self.dof == 0 && self.excess == 0
    }

    /// Returns the combined shortfall, `dof + excess`, which is zero exactly for a minimally
    /// rigid framework. A single scalar lets a caller gate on one comparison.
    pub fn flaw(&self) -> usize {
        self.dof + self.excess
    }
}

/// Computes the generic 2-D rigidity of the framework whose joints are `nodes` and whose
/// bars are the undirected `edges`, each an index pair into `nodes`.
///
/// The coordinates carried by `nodes` do not affect the result; only the joint count and the
/// bar list do (see the module header). Bars may repeat -- a second bar between the same two
/// joints is always redundant and shows up in `excess`.
///
/// Fails when a bar names a joint index outside `nodes`, or joins a joint to itself.
pub fn analyse(nodes: &[Pt], edges: &[(usize, usize)]) -> Outcome<Rigidity> {
    let n = nodes.len();
    for &(a, b) in edges {
        if a >= n || b >= n {
            return Err(err!(
                "Bar ({}, {}) references a joint index outside the {} joints supplied.",
                a, b, n;
            Invalid, Input, Range));
        }
        if a == b {
            return Err(err!(
                "Bar joins joint {} to itself, which is not a bar.", a;
            Invalid, Input));
        }
    }

    let mut game = PebbleGame::new(n);
    let mut rank = 0;
    for &(a, b) in edges {
        if game.admit(a, b) {
            rank += 1;
        }
    }

    let present	= edges.len();
    let budget	= (2 * n).saturating_sub(3);         // 2n - 3, the minimally rigid bar count
    let dof	= budget.saturating_sub(rank);      // rank <= budget for n >= 2, exact there
    let excess	= present.saturating_sub(rank);     // rank <= present always, exact

    Ok(Rigidity { rank, dof, excess })
}

/// The `(2, 3)`-pebble game over a directed orientation of the accepted bars.
///
/// The invariant is `pebbles[v] = PEBBLES_PER_JOINT - outdegree(v)`: each joint starts with
/// its full complement and spends one pebble for each bar oriented out of it. A bar is
/// independent exactly when its endpoints can gather [`ADMIT_THRESHOLD`] pebbles between
/// them, pulling free pebbles inwards by reversing directed paths.
struct PebbleGame {
    pebbles:	Vec<usize>,         // free pebbles at each joint
    out:	Vec<Vec<usize>>,    // out-neighbours in the current orientation
}

impl PebbleGame {
    fn new(n: usize) -> Self {
        Self {
            pebbles:	vec![PEBBLES_PER_JOINT; n],
            out:	vec![Vec::new(); n],
        }
    }

    /// Tries to admit the bar `(u, v)`, returning whether it was independent.
    ///
    /// On acceptance the bar is oriented out of an endpoint that holds a pebble, spending it.
    fn admit(&mut self, u: usize, v: usize) -> bool {
        while self.pebbles[u] + self.pebbles[v] < ADMIT_THRESHOLD {
            // Pull one external pebble to either endpoint; stop when neither can find one.
            if !self.collect(u, u, v) && !self.collect(v, u, v) {
                break;
            }
        }
        if self.pebbles[u] + self.pebbles[v] >= ADMIT_THRESHOLD {
            if self.pebbles[u] > 0 {
                self.pebbles[u] -= 1;
                self.out[u].push(v);
            } else {
                self.pebbles[v] -= 1;
                self.out[v].push(u);
            }
            true
        } else {
            false
        }
    }

    /// Reverses a directed path from `from` to some free pebble, bringing one pebble to
    /// `from`. The two endpoints of the bar under test, `ban_a` and `ban_b`, are excluded as
    /// sources so that each success strictly raises the pebble total on those endpoints.
    /// Returns whether a pebble was brought.
    fn collect(&mut self, from: usize, ban_a: usize, ban_b: usize) -> bool {
        let n = self.pebbles.len();
        let mut parent	= vec![usize::MAX; n];
        let mut seen	= vec![false; n];
        let mut stack	= vec![from];
        seen[from] = true;
        let mut found = None;
        while let Some(v) = stack.pop() {
            if self.pebbles[v] > 0 && v != ban_a && v != ban_b {
                found = Some(v);
                break;
            }
            for &w in &self.out[v] {
                if !seen[w] {
                    seen[w]		= true;
                    parent[w]	= v;
                    stack.push(w);
                }
            }
        }
        match found {
            None => false,
            Some(target) => {
                // The pebble moves from `target` to `from`; the path between them flips, so
                // every intermediate joint keeps its outdegree and only the ends change.
                self.pebbles[target]	-= 1;
                self.pebbles[from]	+= 1;
                let mut w = target;
                while w != from {
                    let p = parent[w];
                    if let Some(pos) = self.out[p].iter().position(|&z| z == w) {
                        self.out[p].swap_remove(pos);
                    }
                    self.out[w].push(p);
                    w = p;
                }
                true
            },
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    // A grid of joint positions; the coordinates are immaterial to the generic result, so
    // any general-position layout serves.
    fn joints(n: usize) -> Vec<Pt> {
        (0..n).map(|i| Pt::new(i as f64, (i * i) as f64)).collect()
    }

    #[test]
    fn triangle_is_minimally_rigid() -> Outcome<()> {
        // Three joints, three bars: the smallest rigid framework.
        let r = res!(analyse(&joints(3), &[(0, 1), (1, 2), (0, 2)]));
        assert_eq!(r.rank, 3);
        assert_eq!(r.dof, 0);
        assert_eq!(r.excess, 0);
        assert!(r.is_minimally_rigid());
        Ok(())
    }

    #[test]
    fn quadrilateral_is_a_mechanism() -> Outcome<()> {
        // A four-bar loop has 2n - 4 bars and flexes with one degree of freedom.
        let r = res!(analyse(&joints(4), &[(0, 1), (1, 2), (2, 3), (3, 0)]));
        assert_eq!(r.rank, 4);
        assert_eq!(r.dof, 1);
        assert_eq!(r.excess, 0);
        assert!(!r.is_rigid());
        Ok(())
    }

    #[test]
    fn quadrilateral_with_one_diagonal_is_minimally_rigid() -> Outcome<()> {
        // The single diagonal triangulates the loop: 2n - 3 = 5 bars, no freedom, no waste.
        let r = res!(analyse(&joints(4), &[(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]));
        assert_eq!(r.rank, 5);
        assert_eq!(r.dof, 0);
        assert_eq!(r.excess, 0);
        assert!(r.is_minimally_rigid());
        Ok(())
    }

    #[test]
    fn quadrilateral_with_both_diagonals_is_over_braced() -> Outcome<()> {
        // The second diagonal adds no stiffness: still rigid, but one redundant bar.
        let r = res!(analyse(
            &joints(4),
            &[(0, 1), (1, 2), (2, 3), (3, 0), (0, 2), (1, 3)],
        ));
        assert_eq!(r.rank, 5);
        assert_eq!(r.dof, 0);
        assert_eq!(r.excess, 1);
        assert!(r.is_rigid());
        assert!(!r.is_minimally_rigid());
        Ok(())
    }

    #[test]
    fn repeated_bar_is_redundant() -> Outcome<()> {
        // A doubled bar between the same joints can never add stiffness.
        let r = res!(analyse(&joints(3), &[(0, 1), (1, 2), (0, 2), (0, 1)]));
        assert_eq!(r.rank, 3);
        assert_eq!(r.dof, 0);
        assert_eq!(r.excess, 1);
        Ok(())
    }

    #[test]
    fn right_bar_count_wrong_distribution_is_not_rigid() -> Outcome<()> {
        // The adversarial case that defeats a bare edge count. Six joints and 2n - 3 = 9
        // bars, so counting alone would declare it minimally rigid. But the bars are
        // misplaced: joints 0..3 form a K4 (six bars where five suffice, one redundant),
        // while the triangle on joints 3, 4, 5 hangs off joint 3 by a single shared joint --
        // a hinge that lets the two rigid pieces rotate about one another. The framework is a
        // one-freedom mechanism carrying a redundant bar, not a rigid structure.
        let edges = [
            // K4 on {0, 1, 2, 3}: over-braced.
            (0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3),
            // Triangle on {3, 4, 5}: rigid, but joined only at joint 3.
            (3, 4), (4, 5), (3, 5),
        ];
        assert_eq!(edges.len(), 2 * 6 - 3);         // the count-only trap: exactly 2n - 3
        let r = res!(analyse(&joints(6), &edges));
        assert_eq!(r.rank, 8);
        assert_eq!(r.dof, 1);                       // Laman catches the hidden mechanism
        assert_eq!(r.excess, 1);                    // and the hidden redundancy
        assert!(!r.is_rigid());
        assert!(!r.is_minimally_rigid());
        Ok(())
    }

    #[test]
    fn out_of_range_bar_is_refused() {
        // Joint index 5 does not exist among three joints.
        let res = analyse(&joints(3), &[(0, 1), (1, 5)]);
        assert!(res.is_err());
    }

    #[test]
    fn self_bar_is_refused() {
        // A bar from a joint to itself is not a bar.
        let res = analyse(&joints(3), &[(0, 0)]);
        assert!(res.is_err());
    }
}
