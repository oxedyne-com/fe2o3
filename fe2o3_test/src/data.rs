use oxedize_fe2o3_core::prelude::*;

use rand::{
    prelude::*,
    distributions::Standard,
    Rng,
    seq::SliceRandom,
};

#[derive(Debug)]
pub enum DataArrangement {
    PlainFill(usize),                   // 12345 n=5
    FillCopy{ n: usize, rep: usize },   // 111112222233333 n=5 rep=3
    RepeatFill{ n: usize, rep: usize }, // 123451234512345 n=5 rep=3
    RepeatFillShuffled{ n: usize, rep: usize }, // 351244315243125 n=5 rep=3, i.e. 5 values, repeated 3 times, but shuffled
    RepeatFillAndSeq{ n: usize, rep: usize, specbox: Box<DataSpec> },   // 123ab456ab789ab n=3 rep=3 specbox.n=2 
}

impl DataArrangement {
    pub fn len(&self) -> Outcome<usize> {
        match self {
            Self::PlainFill(n) => Ok(*n),
            Self::FillCopy{ n, rep }            |
            Self::RepeatFill{ n, rep }          |
            Self::RepeatFillShuffled{ n, rep }  |
            Self::RepeatFillAndSeq{ n, rep, ..} =>
                match n.checked_mul(*rep) {
                    Some(prod) => Ok(prod),
                    None => Err(err!(
                        "Product of n = {} and rep = {} produces usize overflow.", n, rep;
                    Overflow, Integer)),
                },
        }
    }
}

#[derive(Debug)]
pub enum DataFill {
    Const(u8),
    Random,
}

#[derive(Debug)]
pub enum DataSize {
    Const(usize),
    RandUniform{ lo: usize, hi: usize },
    RandNorm{ lo: usize, hi: usize }, // assume 6 stdev in lo..hi
}

impl DataSize {
    pub fn value(&self, fill: &DataFill) -> Vec<u8> {
        let mut rng = rand::thread_rng();
        let len = match self {
            DataSize::Const(len) => {
                *len
            },
            DataSize::RandUniform { lo, hi } => {
                rng.gen_range(*lo..*hi)
            },
            DataSize::RandNorm { lo, hi } => {
                let val: f32 = StdRng::from_entropy().sample(Standard);
                ((((*hi as f32) - (*lo as f32)) * val) as usize) + *lo 
            },
        };
        let mut result = Vec::new();
        for _ in 0..len {
            result.push(match fill {
                DataFill::Const(v) => *v,
                DataFill::Random => rng.gen::<u8>(),
            });
        }
        result
    }
}

/// Create a collection of byte vectors for use in testing.
#[derive(Debug)]
pub struct DataSpec {
    pub size:   DataSize,
    pub fill:   DataFill,
    pub arr:    DataArrangement,
}

impl DataSpec {

    pub fn len(&self) -> Outcome<usize> {
        self.arr.len()
    }

    pub fn generate(&self) -> Outcome<Vec<Vec<u8>>> {
        let mut vals = Vec::new();
        match &self.arr {
            DataArrangement::PlainFill(n) => {
                for _ in 0..*n {
                    let v = self.size.value(&self.fill);
                    vals.push(v);
                }
            },
            DataArrangement::FillCopy{ n, rep } => {
                res!(self.common_checks(n, rep));
                for _ in 0..*rep {
                    let v = self.size.value(&self.fill);
                    for _ in 0..*n {
                        vals.push(v.clone());
                    }
                }
            },
            DataArrangement::RepeatFill{n, rep} => {
                res!(self.common_checks(n, rep));
                let mut vt = Vec::new();
                for _ in 0..*n {
                    let v = self.size.value(&self.fill);
                    vt.push(v);
                }
                for _ in 0..*rep {
                    vals.extend_from_slice(&vt[..]);
                }
            },
            DataArrangement::RepeatFillShuffled{n, rep} => {
                res!(self.common_checks(n, rep));
                let mut rng = thread_rng();
                let mut vt = Vec::new();
                for _ in 0..*n {
                    let v = self.size.value(&self.fill);
                    vt.push(v);
                }
                let mut ind: Vec<usize> = (0..*n).collect();
                for _ in 0..*rep {
                    let _ = &ind.shuffle(&mut rng);
                    for i in &ind {
                        vals.push(vt[*i].clone());
                    }
                }
            },
            DataArrangement::RepeatFillAndSeq{n, rep, specbox} => {
                res!(self.common_checks(n, rep));
                let seq = res!(specbox.generate());
                let s = seq.len();
                if s > *n {
                    return Err(err!(
                        "The given sequence length, {}, must not exceed n, {}.", s, n;
                    Invalid, Input));
                }
                for _ in 0..*rep {
                    for _ in 0..(n - s) {
                        let v = self.size.value(&self.fill);
                        vals.push(v);
                    }
                    for seqitem in &seq {
                        vals.push(seqitem.clone());
                    }
                }
            },
        }
        Ok(vals)
    }

    fn common_checks(&self, _n: &usize, rep: &usize) -> Outcome<()> {
        if *rep == 0 {
            return Err(err!(
                "Given repetitions, {}, must be > 0.", rep;
            Invalid, Input));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_fill_const() -> Outcome<()> {
        let n0 = 5;
        let len0 = 4;
        let spec = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::PlainFill(n0),
            fill:   DataFill::Const(42),
        };
        let data = res!(spec.generate());
        assert_eq!(data.len(), n0);
        let v_expected = vec![42u8; len0];
        for v in data {
            assert_eq!(v, v_expected);
        }
        Ok(())
    }

    #[test]
    fn test_plain_fill_rand() -> Outcome<()> {
        let n0 = 5;
        let len0 = 4;
        let spec = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::PlainFill(n0),
            fill:   DataFill::Random,
        };
        let data = res!(spec.generate());
        assert_eq!(data.len(), n0);
        for v in data {
            assert_eq!(v.len(), len0);
        }
        Ok(())
    }

    #[test]
    fn test_fill_copy_rand() -> Outcome<()> {
        let n0 = 5;
        let rep0 = 3;
        let len0 = 4;
        let spec = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::FillCopy{ n: n0, rep: rep0 },
            fill:   DataFill::Random,
        };
        let data = res!(spec.generate());
        //msg!("FillCopy data:");
        //for val in &data {
        //    msg!("{:02x?}", val);
        //}
        let total = n0 * rep0;
        assert_eq!(data.len(), total);
        let mut v0 = Vec::new();
        for (i, v) in data.iter().enumerate() {
            if i % n0 == 0 {
                v0 = v.clone();
            }
            assert_eq!(v.len(), v0.len());
            for (j, v1) in v.iter().enumerate() {
                assert_eq!(*v1, v0[j]);
            }
        }
        Ok(())
    }

    #[test]
    fn test_repeat_fill_rand() -> Outcome<()> {
        let n0 = 5;
        let rep0 = 3;
        let len0 = 4;
        let spec = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::RepeatFill{ n: n0, rep: rep0 },
            fill:   DataFill::Random,
        };
        let data = res!(spec.generate());
        //msg!("RepeatFill data:");
        //for val in &data {
        //    msg!("{:02x?}", val);
        //}
        let total = n0 * rep0;
        assert_eq!(data.len(), total);
        for (i, v) in data.iter().enumerate() {
            if i < total - n0 {
                assert_eq!(v.len(), data[i + n0].len());
                for (j, v1) in v.iter().enumerate() {
                    assert_eq!(*v1, data[i + n0][j]);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn test_repeat_fill_shuffled_rand() -> Outcome<()> {
        let n0 = 5;
        let rep0 = 3;
        let len0 = 4;
        let spec = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::RepeatFillShuffled{ n: n0, rep: rep0 },
            fill:   DataFill::Random,
        };
        let data = res!(spec.generate());
        //msg!("RepeatFillShuffled data:");
        //for val in &data {
        //    msg!("{:02x?}", val);
        //}
        let total = n0 * rep0;
        assert_eq!(data.len(), total);
        // Loop through the data to make total^2 comparisons ensuring that each repeats rep0
        // times.
        for v1 in &data {
            let mut count = 0;
            for v2 in &data {
                if (*v1).len() == (*v2).len() {
                    let mut diff: bool = false;
                    for k in 0..(*v1).len() {
                        if (*v1)[k] != (*v2)[k] { diff = true; }
                    }
                    if !diff {
                        count += 1;
                    }
                }
            }
            assert_eq!(count, rep0);
        }
        Ok(())
    }

    #[test]
    fn test_repeat_fill_and_seq_rand() -> Outcome<()> {
        let n0 = 5;
        let n1 = 2;
        let rep0 = 3;
        let len0 = 4;
        let spec0 = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::PlainFill(n1),
            fill:   DataFill::Random,
        };
        let spec = DataSpec{
            size:   DataSize::Const(len0),
            arr:    DataArrangement::RepeatFillAndSeq{ n: n0, rep: rep0, specbox: Box::new(spec0) },
            fill:   DataFill::Random,
        };
        let data = res!(spec.generate());
        msg!("RepeatFillAndSeq data:");
        for val in &data {
            msg!("{:02x?}", val);
        }
        let total = n0 * rep0;
        assert_eq!(data.len(), total);
        for (i, v) in data.iter().enumerate() {
            if i < total - n0 && i % n0 > (n0 - n1) {
                assert_eq!(v.len(), data[i + n0].len());
                for (j, v1) in v.iter().enumerate() {
                    assert_eq!(*v1, data[i + n0][j]);
                }
            }
        }
        Ok(())
    }

}
