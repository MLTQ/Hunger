use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub struct RuntimeControl {
    paused: Arc<AtomicBool>,
}

impl RuntimeControl {
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Relaxed);
    }

    pub fn toggle(&self) {
        self.paused.fetch_xor(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeControl;

    #[test]
    fn toggle_flips_pause_state() {
        let control = RuntimeControl::default();
        assert!(!control.is_paused());
        control.toggle();
        assert!(control.is_paused());
        control.toggle();
        assert!(!control.is_paused());
    }
}
