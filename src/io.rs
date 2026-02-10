pub type IoToken = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoWaitKind {
    Readable,
}
