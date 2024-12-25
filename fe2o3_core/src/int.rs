pub trait Bound {
    fn max_size() -> Self;
    fn min_size() -> Self;
}

macro_rules! impl_bound_for_ints {
    ($(($t:ty, $max:expr, $min:expr)),+) => {
        $(
            impl Bound for $t {
                fn max_size() -> Self {
                    $max
                }

                fn min_size() -> Self {
                    $min
                }
            }
        )+
    };
}

impl_bound_for_ints!(
    (i8,    i8::MAX,    i8::MIN),
    (i16,   i16::MAX,   i16::MIN),
    (i32,   i32::MAX,   i32::MIN),
    (i64,   i64::MAX,   i64::MIN),
    (i128,  i128::MAX,  i128::MIN),
    (u8,    u8::MAX,    u8::MIN),
    (u16,   u16::MAX,   u16::MIN),
    (u32,   u32::MAX,   u32::MIN),
    (u64,   u64::MAX,   u64::MIN),
    (u128,  u128::MAX,  u128::MIN),
    (isize, isize::MAX, isize::MIN),
    (usize, usize::MAX, usize::MIN)
);

pub trait One {
    fn one() -> Self;
}

pub trait Zero {
    fn zero() -> Self;
}

macro_rules! impl_one_zero_for_ints {
    ($($t:ty),+) => {
        $(
            impl One for $t {
                fn one() -> Self {
                    1
                }
            }

            impl Zero for $t {
                fn zero() -> Self {
                    0
                }
            }
        )+
    };
}

impl_one_zero_for_ints!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, isize, usize);
