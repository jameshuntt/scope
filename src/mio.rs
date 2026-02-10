// src/io_mio.rs
#![cfg(feature = "mio")]

use crate::{io::IoToken, scope::Scope};
use mio::{Events, Interest, Poll, Token};
use std::{io, time::Duration};

pub struct MioReactor {
    poll: Poll,
    events: Events,
}

impl MioReactor {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            poll: Poll::new()?,
            events: Events::with_capacity(1024),
        })
    }

    pub fn registry(&self) -> &mio::Registry {
        self.poll.registry()
    }

    pub fn register<S: mio::event::Source + ?Sized>(
        &self,
        source: &mut S,
        token: IoToken,
        interest: Interest,
    ) -> io::Result<()> {
        self.poll.registry().register(source, Token(token), interest)
    }

    pub fn poll_and_notify<R>(
        &mut self,
        scope: &mut Scope<'_, R>,
        timeout: Option<Duration>,
    ) -> io::Result<usize> {
        self.poll.poll(&mut self.events, timeout)?;
        let mut n = 0usize;
        for ev in self.events.iter() {
            let tok: IoToken = ev.token().0;
            scope.notify_readable(tok);
            n += 1;
        }
        Ok(n)
    }
}