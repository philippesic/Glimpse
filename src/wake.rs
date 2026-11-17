use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Context(Option<Arc<dyn Fn() + Send + Sync>>);

impl Context {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(notify)))
    }

    pub fn request_repaint(&self) {
        if let Some(notify) = &self.0 {
            notify();
        }
    }
}
