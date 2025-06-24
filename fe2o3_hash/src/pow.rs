pub use oxedyne_fe2o3_core::{
    prelude::*,
    alt::Gnomon,
    //bot::CtrlMsg,
    channels::{
        Recv,
        simplex,
        Simplex,
    },
    rand::Rand,
    thread::{
        Semaphore,
        Sentinel,
        thread_channel,
    },
};
use oxedyne_fe2o3_iop_hash::api::Hasher;

use std::{
    thread,
    time::{
        SystemTime,
        Duration,
    },
};

use num_cpus;

#[derive(Clone, Debug, Default)]
pub struct PowResult {
    pub nonce:      Option<Vec<u8>>,
    pub hash:       Option<Vec<u8>>,
    pub ithread:    usize,
    pub count:      usize,
    pub elapsed:    Duration,
}
    
//#[derive(Clone, Debug, Default)]
#[derive(Clone, Debug)]
pub struct PowSearchResult {
    pub found:          bool,
    pub artefact:       Vec<u8>,
    pub hash_len:       usize,
    pub ncpus:          usize,
    pub total_count:    usize,
    pub elapsed:        Duration,
    pub err:            Option<Error<ErrTag>>,
}
    
/// A hash takes an input called the "pre-image".  For a proof of work this is made up of what we
/// call the "pow pre-image", consisting of an unchanging "pristine" and the nonce, which is varied
/// in order to achieve a hash with the desired properties (conventionally a given number of
/// leading zero bits).
pub trait Pristine<
    const P0: usize,
    const P1: usize,
>:
    Clone
    + Send
    + Sync
{
    const PREFIX_BYTE_LEN: usize = P0;
    const BYTE_LEN: usize = P1;
    fn len(&self) -> usize { Self::BYTE_LEN }
    fn prefix_len(&self) -> usize { Self::PREFIX_BYTE_LEN }

    /// We can't use `oxedyne_fe2o3_core::bytes::ToBytes` because we want an array output.
    fn to_bytes(&self) -> Outcome<[u8; P1]>;
    fn prefix(&self, byts: &mut [u8]) -> Outcome<usize>;
    fn timestamp_valid(&self, artefact: &[u8]) -> Outcome<bool>;
}

#[derive(Clone, Debug)]
pub enum PowMsg {
    Result(Outcome<PowResult>),
    TimedOut(PowResult),
}

#[derive(Clone, Debug)]
pub struct PowCreateParams<
    const P0: usize,
    const P1: usize,
    PRIS: Pristine<P0, P1>,
> {
    pub pvars:      PowVars<P0, P1, PRIS>,
    pub time_lim:   Duration,
    pub count_lim:  usize,
}

/// Capture the required (when receiving) and expected (when sending) Proof of Work parameters.
#[derive(Clone, Debug, Default)]
pub struct PowVars<
    const P0: usize,
    const P1: usize,
    PRIS: Pristine<P0, P1>,
> {
    pub zbits:      ZeroBits,
    pub pristine:   PRIS,  
}

/// Proof of work scheme using a given hash scheme.
#[derive(Clone, Debug)]
pub struct ProofOfWork<
    H: Hasher
    //+ Send
    //+ 'static
> {
    pub hasher: H,
    pub len: usize,
}

pub type ZeroBits = u16;

impl<
    H: Hasher + Send + 'static,
>
    ProofOfWork<H>
{

    pub fn new(hasher: H) -> Outcome<Self> {
        if hasher.is_identity() {
            return Err(err!(
                "The identity hash is not suitable for a proof of work.";
            Invalid, Input));
        }
        let len = match hasher.hash_length() {
            Gnomon::Known(len) => len,
            _ => return Err(err!(
                "The hash length is not known a priori.";
            Invalid, Input)),
        };
        Ok(Self { hasher, len })
    }

    /// Launch the worker and if we can't send the result via the channel issue a last, desperate
    /// message to the screen.
    pub fn work_launcher<
        const N: usize, // Input length.
        const P1: usize, // Pristine length.
    >(
        hasher:     H,
        pristine:   &[u8; P1],
        zbits:      ZeroBits,
        time_lim:   Duration,
        count_lim:  usize,
        ithread:    usize,
        channel:    Simplex<PowMsg>,
        semaphore:  Semaphore,
    ) {
        let result = Self::work::<N, P1>(
            hasher,
            pristine,
            zbits,
            time_lim,
            count_lim,
            ithread,
            semaphore,
        );
        match channel.send(PowMsg::Result(result)) {
            Err(e) => debug!("Proof of work worker thread {}: {}", ithread, e),
            Ok(_) => (),
        }
    }

    /// Search until a hash is found with the number of consecutive zeros at the least significant
    /// end matching at least `ZeroBits` bits, unless the given count or time limits are exceeded.
    /// The given `pristine` and a random nonce make up a input sequence of bytes that is hashed.
    /// The nonce, loop count and hash is returned.
    #[allow(unused_assignments)]
    pub fn work<
        const N: usize, // Input length.
        const P1: usize, // Pristine length.
    >(
        hasher:     H,
        pristine:   &[u8; P1],
        zbits:      ZeroBits,
        time_lim:   Duration,
        count_lim:  usize,
        ithread:    usize,
        semaphore:  Semaphore,
    )
        -> Outcome<PowResult>
    {
        if zbits == 0 {
            return Err(err!(
                "You probably want to the number of zero bits to be more than zero.";
            Invalid, Input));
        }
        if N <= P1 {
            return Err(err!(
                "No room left for nonce with a pristine length {} and a specified input \
                length of {}.  Make pristine smaller, or input length longer.", P1, N;
            Invalid, Input));
        }
        let zbits = zbits as usize;
        let hash_len = match hasher.hash_length() {
            Gnomon::Known(len) => len,
            _ => return Err(err!(
                "The hash length is not known a priori.";
            Invalid, Input)),
        };
        if zbits > 8 * hash_len {
            return Err(err!(
                "The number of zero bits specified, {}, exceeds the hash size of {} bits.",
                zbits, 8 * hash_len;
            Invalid, Input));
        }
        let mut input = [0u8; N];
        let zbyts = zbits / 8;
        let zbits = zbits % 8;
        let mask = if zbits == 0 {
            0
        } else {
            !(u8::MAX << zbits)
        };
        for i in 0..P1 {
            input[i] = pristine[i];
        }
        let mut elapsed = Duration::default();
        let mut count = 0;
        let start = SystemTime::now();
        while semaphore.is_alive() {
            let hasher_clone = hasher.clone();
            Rand::fill_u8(&mut input[P1..]);

            let mut found = true;
            let hash = hasher_clone.hash(&[&input], []).as_vec();
            // Compare zero bytes.
            for z in 0..zbyts {
                if hash[hash.len() - z - 1] != 0 {
                    found = false;
                    continue;
                }
            }
            // Compare remaining zero bits.
            if zbits > 0 {
                if (hash[hash.len() - zbyts - 1] & mask) != 0 {
                    found = false;
                    continue;
                }
            }

            count += 1;
            if count > count_lim {
                break;
            }
            if count % 1_000 == 0 {
                elapsed = res!(start.elapsed());
                if elapsed > time_lim {
                    break;
                }
            }
            if found {
                elapsed = res!(start.elapsed());
                let pow_res = PowResult {
                    nonce:  Some(input[P1..].to_vec()),
                    hash: Some(hash),
                    ithread,
                    count,
                    elapsed,
                };
                return Ok(pow_res);
            }
        }
        
        elapsed = res!(start.elapsed());
        let pow_res = PowResult {
            nonce:  None,
            hash:   None,
            ithread,
            count,
            elapsed,
        };
        Ok(pow_res)
    }

    /// The `ProofOfWork` hasher must produce a hash that:
    /// - contains the minimum number of leading zero bits, and 
    /// - matches the given hash.
    pub fn validate_work<
        //const S: usize,
    >(
        &self,
        pristine_prefix:    &[u8],
        artefact_hash:      &[u8],
        artefact_prefix:    &[u8],
        zbits:              ZeroBits,
        //salt:               [u8; S],
    )
        -> Outcome<bool>
    {
        debug!("");
        let zbits = zbits as usize;
        let zbyts = zbits / 8;
        let zbits = zbits % 8;
        let mask = if zbits == 0 {
            0
        } else {
            !(u8::MAX << zbits)
        };

        let hasher = self.hasher.clone();
        let hash2 = hasher.hash(
            &[&pristine_prefix, &artefact_prefix],
            [], // A salt is not necessary in proof of work.
        ).as_vec();

        //////// Debugging only
        trace!("pristine prefix  [{:>4}]: {:02x?}", pristine_prefix.len(), pristine_prefix);
        trace!("artefact prefix  [{:>4}]: {:02x?}", artefact_prefix.len(), artefact_prefix);
        trace!("given pow hash   [{:>4}]: {:02x?}", artefact_hash.len(), artefact_hash);
        trace!("validation hash  [{:>4}]: {:02x?}", hash2.len(), hash2);
        ////////

        // Compare zero bytes.
        for z in 0..zbyts {
            if hash2[hash2.len() - z - 1] != 0 {
                return Ok(false);
            }
        }
        // Compare remaining zero bits.
        if zbits > 0 {
            if (hash2[hash2.len() - zbyts - 1] & mask) != 0 {
                return Ok(false);
            }
        }
        if artefact_hash.len() != hash2.len() {
            return Ok(false);
        }
        for (i, b) in artefact_hash.iter().enumerate() {
            if hash2[i] != *b {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// ```ignore
    ///          ___________________ artefact _____________________
    ///         /                                                  \
    ///  ___________________ input N __________________
    /// /                                              \
    ///  ___________ pristine P1 __________
    /// /                                  \
    /// +-------+---------------------------+-----------+-----------+
    /// |       |      pristine artefact    |   nonce   |   hash    |
    /// +-------+---------------------------+-----------+-----------+
    ///  \_____/ \_____________________________________/ \_________/
    ///     |                        |                        | 
    ///  pristine                 artefact                 artefact
    ///  prefix P0                 prefix                    hash
    ///
    ///  ```
    pub fn create<
        const N: usize, // Input length.
        const P0: usize, // Pristine prefix length.
        const P1: usize, // Pristine length.
        PRIS: Pristine<P0, P1>,
    >(
        &self,
        params: &PowCreateParams<P0, P1, PRIS>,
    )
        -> Outcome<PowSearchResult>
    {
        let pristine = res!(params.pvars.pristine.to_bytes());
        
        // Launch work threads to discover a valid nonce.
        let ncpus = num_cpus::get();
        let mut sentinels = Vec::new();
        let channel = simplex();
        let mut pow_res = PowResult::default();
        let mut errs = Vec::new();
        let params = params.clone();
        for i in 0..ncpus {
            let (semaphore, sentinel) = thread_channel();
            sentinels.push(sentinel);
            let channel_clone = channel.clone();
            let hasher_clone = self.hasher.clone();
            // TODO make into async thread?
            thread::spawn(move || {
                Self::work_launcher::<{N}, {P1}>(
                    hasher_clone,
                    &pristine,
                    params.pvars.zbits,
                    params.time_lim,
                    params.count_lim,
                    i,
                    channel_clone,
                    semaphore,
                );
            });
        }

        // Periodically check on thread status and stop remaining threads if one has finished.
        let mut total_count: usize = 0;
        let mut waiting_for = ncpus;
        let mut found = false;
        while waiting_for > 0 {
            match channel.try_recv() {
                Recv::Empty => (),
                Recv::Result(res) => {
                    waiting_for -= 1;
                    match res {
                        Err(e) | Ok(PowMsg::Result(Err(e))) => errs.push(Box::new(e)),
                        Ok(PowMsg::Result(Ok(res))) => {
                            if res.nonce.is_some() {
                                match total_count.checked_add(res.count) {
                                    Some(sum) => total_count = sum,
                                    None => debug!(
                                        "Thread {}: Attempt to add count of {} to total count of \
                                        {} for successful thread failed due to overflow.",
                                        res.ithread, res.count, total_count,
                                    ),
                                }
                                pow_res = res;
                                found = true;
                                for j in 0..ncpus {
                                    if j != pow_res.ithread {
                                        if !sentinels[j].is_finished() {
                                            sentinels[j].stop();
                                        }
                                    }
                                }
                            }
                        },
                        Ok(PowMsg::TimedOut(res)) => {
                            match total_count.checked_add(res.count) {
                                Some(sum) => total_count = sum,
                                None => debug!(
                                    "Thread {}: Attempt to add count of {} to total count of \
                                    {} for unsuccessful thread failed due to overflow.",
                                    res.ithread, res.count, total_count,
                                ),
                            };
                        },
                        //msg => return Err(err!(
                        //    "Unrecognised message {:?} from worker thread.", msg,
                        //), Bug, Unreachable)),
                    }
                },
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        debug!("{:?}", pow_res);
        // Discard the pristine prefix in the publicly visible artefact.
        let start = params.pvars.pristine.prefix_len();
        let mut artefact = pristine[start..].to_vec();
        if let Some(mut nonce) = pow_res.nonce {
            artefact.append(&mut nonce);
        }
        if let Some(mut hash) = pow_res.hash {
            debug!("hash [{}]: {}",hash.len(),
                hash.iter().map(|b| fmt!("{:08b}", b)).collect::<Vec<_>>().join("_"));
            artefact.append(&mut hash);
        }
        Ok(PowSearchResult {
            found,
            artefact,
            hash_len: self.len,
            ncpus,
            total_count,
            elapsed: if found { pow_res.elapsed } else { params.time_lim },
            err: if errs.len() == 0 { None } else { Some(Error::Collection(errs)) },
        })
    }

    pub fn validate<
        const P0: usize,
        const P1: usize,
        //const S: usize,
        PRIS: Pristine<P0, P1>,
    >(
        &self,
        powvars:    &PowVars<P0, P1, PRIS>,
        artefact:   &[u8],
        //salt:       [u8; S],
    )
        -> Outcome<bool>
    {
        if !res!(powvars.pristine.timestamp_valid(&artefact)) {
            debug!("Timestamp invalid");
            return Ok(false);
        }
        let mut pristine_prefix = [0u8; P0];
        res!(powvars.pristine.prefix(&mut pristine_prefix));
        debug!("");
        self.validate_work(
            &pristine_prefix,
            &artefact[artefact.len() - self.len..],
            &artefact[..artefact.len() - self.len],
            powvars.zbits,
            //salt,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::HashScheme;

    #[derive(Clone, Debug)]
    struct TestPristine<const P1: usize> {
        byts: [u8; P1]
    }

    impl<const P1: usize> Default for TestPristine<P1> {
        fn default() -> Self {
            let mut byts = [0; P1]; 
            Rand::fill_u8(&mut byts[..]);
            Self {
                byts,
            }
        }
    }

    impl<
        const P0: usize,
        const P1: usize,
    >
        Pristine<P0, P1> for TestPristine<P1>
    {
        const PREFIX_BYTE_LEN: usize = 0;
        fn to_bytes(&self) -> Outcome<[u8; P1]> { Ok(self.byts) }
        fn prefix(&self, _byts: &mut [u8]) -> Outcome<usize> { Ok(0) }
        fn timestamp_valid(&self, _artefact: &[u8]) -> Outcome<bool> { Ok(true) }
    }

    #[test]
    fn test_concurrent_work() -> Outcome<()> {
        let zbits = 25;
        const P1: usize = 8;
        const NONCE: usize = 8; // Nonce size
        let pristine = TestPristine::<P1>::default();
        let pow = res!(ProofOfWork::new(HashScheme::new_seahash()));
        let pvars = PowVars {
            zbits,
            pristine: pristine.clone(),
        };
        let powp = PowCreateParams {
            pvars: pvars.clone(),
            time_lim: Duration::from_secs(200),
            count_lim: usize::MAX,
        };
        match pow.create::<{P1+NONCE}, 0, P1, TestPristine<P1>>(&powp) {
            Ok(PowSearchResult{
                found,
                artefact,
                hash_len,
                ncpus: _,
                total_count,
                elapsed,
                err,
            }) => {
                if found {
                    debug!("total count = {}", total_count);
                    debug!("elapsed = {:.2?}", elapsed);
                    debug!("pristine = {:02x?}", pristine);
                    debug!("zero bits = {}", zbits);
                    debug!("artefact [{}] = {:02x?}", artefact.len(), artefact);
                    let hash = &artefact[artefact.len() - hash_len..];
                    debug!("hash: ");
                    println!("{:02x?}", hash);
                    for byt in hash {
                        print!(" {:08b}", byt);
                    }
                    println!();
                    if let Some(e) = err {
                        debug!("{}", e);
                    }
                    assert!(res!(pow.validate(
                        &pvars,
                        &artefact,
                    )));
                    // Change a byte and ensure it does not validate.
                    let mut mangled = artefact.clone();
                    mangled[2] = 134;
                    match pow.validate(
                        &pvars,
                        &mangled,
                    ) {
                        Ok(true) => return Err(err!(
                            "The proof of work, with one byte changed, should not have validated.";
                        Input, Invalid)),
                        _ => (),
                    }
                } else {
                    debug!("Proof of work timed out.");
                }
            },
            Err(e) => return Err(e),
        }
        Ok(())
    }
}
