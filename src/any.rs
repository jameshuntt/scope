// src/any.rs
use std::any::Any;

pub type AnyBox = Box<dyn Any + Send>;

#[derive(Debug)]
pub struct TaggedAny<Tag> {
    pub tag: Tag,
    pub value: AnyBox,
}

impl<Tag: Copy> TaggedAny<Tag> {
    #[inline]
    pub fn is<T: Any>(&self) -> bool {
        self.value.is::<T>()
    }

    #[inline]
    pub fn downcast<T: Any>(self) -> Result<(Tag, T), Self> {
        match self.value.downcast::<T>() {
            Ok(v) => Ok((self.tag, *v)),
            Err(v) => Err(Self { tag: self.tag, value: v }),
        }
    }

    #[inline]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.value.downcast_ref::<T>()
    }
}