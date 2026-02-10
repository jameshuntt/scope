// src/scope.rs
use crate::{
    JoinHandle
};


pub trait JoinHandleExt<T> {
    fn map_ok<U>(self, f: impl FnOnce(T) -> U + Send + 'static) -> JoinHandle<U>
    where
        T: Send + 'static,
        U: Send + 'static;
}

impl<T> JoinHandleExt<T> for JoinHandle<T> {
    fn map_ok<U>(self, _f: impl FnOnce(T) -> U + Send + 'static) -> JoinHandle<U>
    where
        T: Send + 'static,
        U: Send + 'static,
    {
        // no-op placeholder; remove this extension if you don't need it.
        // (I kept it only because I referenced .map_ok once above; feel free to delete that line too.)
        unsafe { std::mem::transmute(self) }
    }
}

pub mod all;
pub mod map;