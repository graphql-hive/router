pub struct Capture<S>(Option<S>);

impl<S> Capture<S> {
    pub fn disabled() -> Self {
        Self(None)
    }

    pub fn enabled(state: S) -> Self {
        Self(Some(state))
    }

    pub fn take(self) -> Option<S> {
        self.0
    }

    pub fn as_ref(&self) -> Option<&S> {
        self.0.as_ref()
    }

    pub fn as_mut(&mut self) -> Option<&mut S> {
        self.0.as_mut()
    }
}
