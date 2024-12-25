#[macro_export]
macro_rules! shake128 {
    ( $output:expr ; $( $input:expr ),* ) => {
        let mut hasher = sha3::Shake128::default();
        $(
            digest::Input::process(&mut hasher, $input);
        )*
        let mut reader = digest::ExtendableOutput::xof_result(hasher);
        digest::XofReader::read(&mut reader, $output);
    }
}

#[macro_export]
macro_rules! shake256 {
    ( $output:expr ; $( $input:expr ),* ) => {
        let mut hasher = sha3::Shake256::default();
        $(
            digest::Input::process(&mut hasher, $input);
        )*
        let mut reader = digest::ExtendableOutput::xof_result(hasher);
        digest::XofReader::read(&mut reader, $output);
    }
}

#[macro_export]
macro_rules! polyvec {
    ( $polyvec:ident, $len:expr ) => {

        #[derive(Copy, Clone)]
        pub struct $polyvec(pub [Poly; $len]);

        impl $polyvec {
            pub fn reduce(&mut self) {
                self.0.iter_mut()
                    .for_each(poly::reduce)
            }

            pub fn csubq(&mut self) {
                self.0.iter_mut()
                    .for_each(poly::csubq)
            }

            pub fn freeze(&mut self) {
                self.0.iter_mut()
                    .for_each(poly::freeze)
            }

            pub fn with_add(&mut self, u: &Self, v: &Self) {
                for i in 0..$len {
                    poly::add(&mut self[i], &u[i], &v[i]);
                }
            }

            pub fn add_assign(&mut self, u: &Self) {
                for i in 0..$len {
                    poly::add_assign(&mut self[i], &u[i]);
                }
            }

            pub fn with_sub(&mut self, u: &Self, v: &Self) {
                for i in 0..$len {
                    poly::sub(&mut self[i], &u[i], &v[i]);
                }
            }

            pub fn shift_left(&mut self, k: u32) {
                self.0.iter_mut()
                    .for_each(|p| poly::shift_left(p, k));
            }

            pub fn ntt(&mut self) {
                self.0.iter_mut()
                    .for_each(poly::ntt);
            }

            pub fn invntt_montgomery(&mut self) {
                self.0.iter_mut()
                    .for_each(poly::invntt_montgomery)
            }

            pub fn chknorm(&self, bound: u32) -> bool {
                self.0.iter()
                    .map(|p| poly::chknorm(p, bound))
                    .fold(false, |x, y| x | y)
            }
        }

        impl ::core::ops::Index<usize> for $polyvec {
            type Output = Poly;

            #[inline(always)]
            fn index(&self, i: usize) -> &Self::Output {
                self.0.index(i)
            }
        }

        impl ::core::ops::IndexMut<usize> for $polyvec {
            #[inline(always)]
            fn index_mut(&mut self, i: usize) -> &mut Self::Output {
                self.0.index_mut(i)
            }
        }

        impl ::core::cmp::PartialEq for $polyvec {
            fn eq(&self, other: &Self) -> bool {
                self.0.iter().zip(&other.0)
                    .flat_map(|(x, y)| x.iter().zip(y.iter()))
                    .all(|(x, y)| x == y)
            }
        }

        impl Eq for $polyvec {}

        impl Default for $polyvec {
            fn default() -> Self {
                $polyvec([[0; N]; $len])
            }
        }
    }
}
