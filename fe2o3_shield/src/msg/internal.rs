//use crate::{
//    msg::external::{
//        MsgBuilder,
//    },
//};

use std::{
    fmt::Debug,
};
    
#[derive(Clone, Debug)]
pub enum ServerMsg {
    Finish,
    //Marco(id::Mid),
    //Polo(id::Mid),
    //Msg(MsgBuilder),
    Ready,
}

impl oxedize_fe2o3_bot::msg::BotMsg<oxedize_fe2o3_core::error::ErrTag> for ServerMsg {}

impl oxedize_fe2o3_core::bot::CtrlMsg for ServerMsg {
    fn finish() -> Self { Self::Finish }
    fn ready() -> Self { Self::Ready }
}

