//! Some utilities for testing and performance measurement.

use crate::prelude::*;

pub fn test_it<
    F: Fn() -> Outcome<()> + 'static + Sync + Send
>(
    filter: &str,
    tags: &[&str],
    closure: F,
)
    -> Outcome<()>
{
    if tags.len() == 0 {
        return Err(err!(
            "The test must have at least one tag, being the title.";
        Invalid, Input, Missing, Bug));
    }
    for tag in tags {
        if (*tag).starts_with(filter) {
            test!("'{}' test commencing...", tags[0]);
            res!(closure());
            test!("'{}' test completed.", tags[0]);
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_000() -> Outcome<()> {
        let result = test_it("hello", &["MyTest 000", "hello"], || {
            let n = 42;
            test!("n = {}", n);
            req!(n, 43);
            Ok(())
        });
        test!("result = {:?}", result);
        log_finish_wait!();
        Ok(())
    }
}
