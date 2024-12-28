use oxedize_fe2o3_core::prelude::*;

pub trait OptionRefVec<T> {
    fn with_len<'a>(self, len: usize) -> Outcome<&'a [T]> where Self: 'a;
}

impl<'a, T> OptionRefVec<T> for Option<&'a Vec<T>> {
    fn with_len<'b>(self, len: usize) -> Outcome<&'b [T]> where 'a: 'b {
        self.map_or_else(
            || Err(err!("Expected {} values, found none.", len; Missing, Data)),
            |vals| {
                if vals.len() == len {
                    Ok(vals.as_slice())
                } else {
                    Err(err!(
                        "Expected {} values, found {}.", len, vals.len();
                    Mismatch, Data, Size))
                }
            }
        )
    }
}

pub trait OptionMutVec<T> {
    fn with_len<'a>(&'a mut self, len: usize) -> Outcome<&'a mut [T]> where Self: 'a;
}

impl<'a, T> OptionMutVec<T> for Option<&'a mut Vec<T>> {
    fn with_len<'b>(&'b mut self, len: usize) -> Outcome<&'b mut [T]> where 'a: 'b {
        self.as_deref_mut().map_or_else(
            || Err(err!("Expected {} values, found none.", len; Missing, Data)),
            |vals| {
                let vals = vals.as_mut_slice();
                if vals.len() == len {
                    Ok(vals)
                } else {
                    Err(err!(
                        "Expected {} values, found {}.", len, vals.len();
                    Mismatch, Data, Size))
                }
            }
        )
    }
}
