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






#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn test_on_drop_executes_logic() {
        let counter = AtomicUsize::new(0);
        {
            let _guard = OnDrop::new(|| {
                counter.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(counter.load(Ordering::SeqCst), 0);
        } // _guard dropped here
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_on_drop_manual_drop() {
        let mut triggered = false;
        let guard = OnDrop::new(|| triggered = true);
        drop(guard);
        assert!(triggered);
    }

    #[test]
    fn test_lease_with_atomics() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        
        let resource_load = AtomicUsize::new(0);
        let mut shared_val = String::from("Initial");
    
        {
            // The closure captures the reference to the atomic, 
            // which only requires a shared borrow (&) to mutate the inner value.
            let mut lease = Lease::new(&mut shared_val, || {
                resource_load.fetch_sub(1, Ordering::SeqCst);
            });
            
            resource_load.fetch_add(1, Ordering::SeqCst);
            lease.push_str(" -> Leased");
            
            assert_eq!(resource_load.load(Ordering::SeqCst), 1);
        } 
    
        assert_eq!(resource_load.load(Ordering::SeqCst), 0);
        assert_eq!(shared_val, "Initial -> Leased");
    }
}


