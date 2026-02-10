use std::ops::{Deref, DerefMut};

/// Run a closure when dropped.
pub struct OnDrop<F: FnOnce()> {
    f: Option<F>,
}

impl<F: FnOnce()> OnDrop<F> {
    #[inline]
    pub fn new(f: F) -> Self {
        Self { f: Some(f) }
    }
}

impl<F: FnOnce()> Drop for OnDrop<F> {
    fn drop(&mut self) {
        if let Some(f) = self.f.take() {
            f();
        }
    }
}

/// A borrowed "lease" over a value that triggers a side effect when released.
pub struct Lease<'a, T, F: FnOnce()> {
    value: &'a mut T,
    _on_drop: OnDrop<F>,
}

impl<'a, T, F: FnOnce()> Lease<'a, T, F> {
    #[inline]
    pub fn new(value: &'a mut T, on_drop: F) -> Self {
        Self { value, _on_drop: OnDrop::new(on_drop) }
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<'a, T, F: FnOnce()> Deref for Lease<'a, T, F> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, T, F: FnOnce()> DerefMut for Lease<'a, T, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}
