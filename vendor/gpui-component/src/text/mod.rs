mod document_text;
mod format;
mod inline;
mod node;
mod style;
mod text_view;
mod utils;

use gpui::{App, RenderImage, SharedUri, Window};
use std::{collections::HashMap, sync::Arc};

pub type LinkHandler = dyn Fn(&str, &mut Window, &mut App) + Send + Sync;
pub type LinkLabel = dyn Fn(&str) -> String + Send + Sync;

#[derive(Default, Clone)]
pub struct TextViewResources {
    pub images: HashMap<SharedUri, Arc<RenderImage>>,
    pub restrict_images: bool,
    pub on_link: Option<Arc<LinkHandler>>,
    pub link_label: Option<Arc<LinkLabel>>,
}
pub use style::*;
pub use text_view::*;

pub(crate) fn init(cx: &mut App) {
    text_view::init(cx);
}
