use std::cmp;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum Relation {
    Less,
    Equal,
    Greater,
    Complement, // e.g. equal, but opposite
}

impl Relation {

    pub fn reverse(self) -> Self {
        match self {
            Self::Less => Self::Greater,
            Self::Greater => Self::Less,
            _ => self,
        }
    }

    pub fn ordering(cmp: cmp::Ordering) -> Self {
        match cmp {
            cmp::Ordering::Less => Self::Less,
            cmp::Ordering::Greater => Self::Greater,
            cmp::Ordering::Equal => Self::Equal,
        }
    }

}

#[derive(Clone, Debug)]
pub struct Ordered<V: Ord + PartialOrd, N: Ord + PartialOrd> {
    val: V,
    ord: N,
}

impl<
    V: Ord + PartialOrd,
    N: Ord + PartialOrd,
>
    Ordered<V, N>
{
    pub fn new(val: V, ord: N) -> Self {
        Self {
            val,
            ord,
        }
    }
}

pub fn ord_string(n: usize) -> String {
    let suffix = match n % 10 {
        1 if n % 100 != 11 => "st",
        2 if n % 100 != 12 => "nd",
        3 if n % 100 != 13 => "rd",
        _ => "th",
    };
    format!("{}{}", n, suffix)
}
