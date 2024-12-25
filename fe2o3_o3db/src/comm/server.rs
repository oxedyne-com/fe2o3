use crate::{
    comm::channels::ChannelPool,
};

use oxedize_fe2o3_jdat::id::NumIdDat;

pub struct ServerChannels<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
> {
    chans: ChannelPool<UIDL, UID, ENC, KH>,
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
>
    ServerChannels<UIDL, UID>
{
    pub fn update(&mut self, chans: ChannelPool<UIDL, UID, ENC, KH>) {
        self.chans = chans;
    }
}
