use oxedize_fe2o3_core::{
    prelude::*,
};

pub trait BotMsg<ETAG: GenTag>:
    Clone
    + std::fmt::Debug
    + Send
    where oxedize_fe2o3_core::error::Error<ETAG>: std::error::Error
{
    //fn get_base(&self) -> Option<BaseMsg<ETAG>>;
    //fn get_err(&self) -> Option<Error<ETAG>>;
}
