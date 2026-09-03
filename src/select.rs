//! The two-armed result of `select2`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Which of two tasks finished first, with its result.
pub enum Either<A, B> {
    /// The first task finished first.
    Left(A),
    /// The second task finished first.
    Right(B),
}

impl<A, B> Either<A, B> {
    /// Whether this is the `Left` arm.
    pub fn is_left(&self) -> bool {
        matches!(self, Either::Left(_))
    }
    /// Whether this is the `Right` arm.
    pub fn is_right(&self) -> bool {
        matches!(self, Either::Right(_))
    }
}
