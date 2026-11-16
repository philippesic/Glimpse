//! AppKit source editing hosted inside the Rust-rendered document pane.
use crate::{
    appearance,
    commands::{self, Action, Binding},
    editor,
    storage::Settings,
    syntax,
};
use eframe::egui;
use objc2::{
    ClassType, DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Imp, Sel},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSColor, NSEvent, NSEventMask, NSEventModifierFlags,
    NSFont, NSFontAttributeName, NSFontDescriptorSymbolicTraits, NSForegroundColorAttributeName,
    NSGraphicsContext, NSLayoutManager, NSMenu, NSMenuItem, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSRectClip, NSRectFill, NSRulerOrientation, NSRulerView,
    NSScrollView, NSStringDrawing, NSTextContainer, NSTextStorage, NSTextView, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSDictionary, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect,
    NSSize, NSString, NSStringEncodingConversionOptions, NSUTF8StringEncoding,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

// Copy directly into the Rust buffer instead of creating an autoreleased UTF-8 copy.
fn snapshot(value: &NSString) -> String {
    let length = value.length();
    if length == 0 {
        return String::new();
    }
    let capacity = value.maximumLengthOfBytesUsingEncoding(NSUTF8StringEncoding);
    let mut bytes = Vec::<u8>::with_capacity(capacity);
    let mut used = 0;
    let mut remaining = NSRange::new(0, 0);
    // AppKit initializes at most `capacity` bytes and reports their exact count.
    let copied = unsafe {
        value.getBytes_maxLength_usedLength_encoding_options_range_remainingRange(
            bytes.as_mut_ptr().cast(),
            capacity,
            &mut used,
            NSUTF8StringEncoding,
            NSStringEncodingConversionOptions::empty(),
            NSRange::new(0, length),
            &mut remaining,
        )
    };
    if copied && remaining.length == 0 && used <= capacity {
        unsafe {
            bytes.set_len(used);
        }
        if let Ok(text) = String::from_utf8(bytes) {
            return text;
        }
    }
    value.to_string()
}

struct Ivars {
    current_line: Cell<bool>,
    long_line: Cell<bool>,
    line_color: Cell<egui::Color32>,
    dirty: Cell<bool>,
    drawn: Cell<bool>,
    suppress: Cell<bool>,
    ctx: egui::Context,
    wake: Arc<dyn Fn() + Send + Sync>,
    indent: Cell<usize>,
    tabs: Cell<bool>,
    auto_indent: Cell<bool>,
    bindings: Rc<RefCell<Vec<Binding>>>,
    queue: Rc<RefCell<Vec<Action>>>,
}
define_class!(
    #[unsafe(super(NSTextView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct SourceView;
    impl SourceView {
        #[unsafe(method(drawViewBackgroundInRect:))]
        fn background(&self,rect:NSRect) {
            unsafe{let _:()=msg_send![super(self),drawViewBackgroundInRect:rect];}
            if !self.ivars().current_line.get() || self.ivars().long_line.get() { return; }
            let selected=self.selectedRange();
            if selected.length>0 { return; }
            let Some(layout)=(unsafe{self.layoutManager()}) else {return};
            let length=self.string().length();
            let at_extra=selected.location>=length && layout.extraLineFragmentTextContainer().is_some();
            let mut line=if length==0 || at_extra { layout.extraLineFragmentRect() } else {
                let glyph=layout.glyphIndexForCharacterAtIndex(selected.location.min(length-1));
                unsafe{layout.lineFragmentRectForGlyphAtIndex_effectiveRange(glyph,std::ptr::null_mut())}
            };
            line.origin.y+=self.textContainerOrigin().y;
            line.origin.x=rect.origin.x;line.size.width=rect.size.width;
            if line.size.height<=0. {line.size.height=self.font().map(|font|font.pointSize()*1.3).unwrap_or(18.);}
            let color=self.ivars().line_color.get();
            NSColor::colorWithSRGBRed_green_blue_alpha(color.r() as f64/255.,color.g() as f64/255.,color.b() as f64/255.,0.50).set();
            let _clip=DrawingState::clipped(rect);NSRectFill(line);
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self,rect:NSRect){let _timing=crate::perf::Span::new("native_draw");if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some(){eprintln!("NATIVE_DRAW pass={} active={} key={}",self.ivars().ctx.cumulative_pass_nr(),NSApplication::sharedApplication(MainThreadMarker::from(self)).isActive(),self.window().is_some_and(|w|w.isKeyWindow()));}
            unsafe{let _:()=msg_send![super(self),drawRect:rect];}if !self.ivars().drawn.replace(true){
                if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some() && self.string().length()>0 && let Some(layout)=unsafe{self.layoutManager()} {
                    let glyph=layout.glyphIndexForCharacterAtIndex(0);
                    let line=unsafe{layout.lineFragmentRectForGlyphAtIndex_effectiveRange(glyph,std::ptr::null_mut())};
                    eprintln!("NATIVE_TYPOGRAPHY font_pt={} line_fragment_pt={}",self.font().map(|font|font.pointSize()).unwrap_or(0.),line.size.height);
                }
self.ivars().ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);self.ivars().ctx.request_repaint();}}
        #[unsafe(method(didChangeText))]
        fn changed(&self) {
            unsafe {let _:()=msg_send![super(self),didChangeText];}
            if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some(){eprintln!("NATIVE_CHANGED pass={} viewport={:?}",self.ivars().ctx.cumulative_pass_nr(),self.ivars().ctx.viewport_id());}
            if !self.ivars().suppress.get(){self.ivars().dirty.set(true);}
            self.ivars().ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden); self.ivars().ctx.request_repaint_of(egui::ViewportId::ROOT); (self.ivars().wake)();
        }
        #[unsafe(method(performKeyEquivalent:))]
        fn equivalent(&self,event:&NSEvent)->bool {
            if event_action(event,&self.ivars().bindings.borrow()).is_some(){return true.into();}
            unsafe{msg_send![super(self),performKeyEquivalent:event]}
        }
        #[unsafe(method(keyDown:))]
        fn key(&self,event:&NSEvent){unsafe{let _:()=msg_send![super(self),keyDown:event];}self.setNeedsDisplay(true);self.ivars().ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden); self.ivars().ctx.request_repaint_of(egui::ViewportId::ROOT); (self.ivars().wake)();}
        #[unsafe(method(mouseDown:))]
        fn mouse(&self,event:&NSEvent){unsafe{let _:()=msg_send![super(self),mouseDown:event];}self.setNeedsDisplay(true);self.ivars().ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden); self.ivars().ctx.request_repaint_of(egui::ViewportId::ROOT); (self.ivars().wake)();}
        #[unsafe(method(cancelOperation:))]
        fn cancel(&self,_sender:Option<&AnyObject>){self.ivars().queue.borrow_mut().push(Action::Cancel);self.ivars().ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);self.ivars().ctx.request_repaint();(self.ivars().wake)();}
        #[unsafe(method(insertTab:))]
        fn tab(&self,_sender:Option<&AnyObject>){self.indent(false);}
        #[unsafe(method(insertBacktab:))]
        fn backtab(&self,_sender:Option<&AnyObject>){self.indent(true);}
        #[unsafe(method(insertNewline:))]
        fn newline(&self,sender:Option<&AnyObject>){
            if !self.ivars().auto_indent.get(){unsafe{let _:()=msg_send![super(self),insertNewline:sender];}return;}
            let string=self.string();let selected=self.selectedRange();
            let paragraph=string.paragraphRangeForRange(NSRange::new(selected.location,0));
            let before=string.substringWithRange(NSRange::new(paragraph.location,selected.location-paragraph.location)).to_string();
            let indent:String=before.chars().take_while(|c|matches!(c,' '|'\t')).collect();
            unsafe{let _:()=msg_send![super(self),insertNewline:sender];}
            if !indent.is_empty(){unsafe{let _:()=msg_send![self,insertText:&*NSString::from_str(&indent),replacementRange:self.selectedRange()];}}
        }

    }
    unsafe impl NSObjectProtocol for SourceView {}
);
impl SourceView {
    fn indent(&self, outdent: bool) {
        let string = self.string();
        let selected = self.selectedRange();
        let unit = if self.ivars().tabs.get() {
            "\t".into()
        } else {
            " ".repeat(self.ivars().indent.get())
        };
        let selected_text = string.substringWithRange(selected).to_string();
        if !outdent && !selected_text.contains('\n') {
            unsafe {
                let _: () = msg_send![self,insertText:&*NSString::from_str(&unit),replacementRange:selected];
            }
            return;
        }
        let end = if selected.length > 0 && selected_text.ends_with('\n') {
            selected.length - 1
        } else {
            selected.length
        };
        let range = string.paragraphRangeForRange(NSRange::new(selected.location, end));
        let old = string.substringWithRange(range).to_string();
        let new = editor::indent_lines(&old, &unit, outdent);
        let length = new.encode_utf16().count();
        unsafe {
            let _: () =
                msg_send![self,insertText:&*NSString::from_str(&new),replacementRange:range];
        }
        self.setSelectedRange(NSRange::new(range.location, length));
    }
}
struct CommandIvars {
    queue: Rc<RefCell<Vec<Action>>>,
    ctx: egui::Context,
    wake: Arc<dyn Fn() + Send + Sync>,
}
define_class!(
 #[unsafe(super(NSObject))]
 #[thread_kind=MainThreadOnly]
 #[ivars=CommandIvars]
 struct CommandTarget;
 impl CommandTarget{
  #[unsafe(method(invoke:))]
  fn invoke(&self,sender:&NSMenuItem){if let Some(command)=commands::COMMANDS.get(sender.tag() as usize){self.ivars().queue.borrow_mut().push(command.action);self.ivars().ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);self.ivars().ctx.request_repaint();(self.ivars().wake)();}}
 }
 unsafe impl NSObjectProtocol for CommandTarget{}
);
struct RulerIvars {
    text: Retained<SourceView>,
    starts: RefCell<Arc<Vec<usize>>>,
    empty_baseline: Cell<f64>,
    labels: RefCell<Option<DigitLabels>>,
    padding: Cell<f64>,
}
struct DrawingState;
impl DrawingState {
    fn clipped(bounds: NSRect) -> Self {
        NSGraphicsContext::saveGraphicsState_class();
        NSRectClip(bounds);
        Self
    }
}
impl Drop for DrawingState {
    fn drop(&mut self) {
        NSGraphicsContext::restoreGraphicsState_class();
    }
}
define_class!(
    #[unsafe(super(NSRulerView))]
    #[thread_kind=MainThreadOnly]
    #[ivars=RulerIvars]
    struct LineRuler;
    impl LineRuler {
        #[unsafe(method(drawRect:))]
        fn draw(&self,rect:NSRect){
            // AppKit also paints the separator outside the hash-mark hook.
            let _state=DrawingState::clipped(self.bounds());
            unsafe{let _:()=msg_send![super(self),drawRect:rect];}
        }
        #[unsafe(method(isFlipped))]
        fn flipped(&self)->bool { true }
        #[unsafe(method(drawHashMarksAndLabelsInRect:))]
        fn draw_numbers(&self,_rect:NSRect){
            let _timing=crate::perf::Span::new("ruler_draw");
            // NSRulerView's dirty rectangle can include neighboring host surfaces.
            let bounds=self.bounds();
            let _state=DrawingState::clipped(bounds);
            let view=&self.ivars().text;
            view.backgroundColor().set();NSRectFill(bounds);
            if view.ivars().long_line.get() { return; }
            let Some(layout)=(unsafe{view.layoutManager()}) else{return};
            let Some(container)=(unsafe{view.textContainer()}) else{return};
            let container_origin=view.textContainerOrigin();
            let mut visible=view.visibleRect();
            visible.origin.x-=container_origin.x;
            visible.origin.y-=container_origin.y;
            let glyphs=layout.glyphRangeForBoundingRect_inTextContainer(visible,&container);
            let chars=unsafe{layout.characterRangeForGlyphRange_actualGlyphRange(glyphs,std::ptr::null_mut())};
            let starts=self.ivars().starts.borrow();
            let first=starts.partition_point(|n|*n<=chars.location).saturating_sub(1);
            let length=view.string().length();
            let rows:Vec<_>=starts.iter().enumerate().skip(first)
                .take_while(|(_,start)|**start<=chars.location+chars.length && **start<=length)
                .collect();
            if rows.is_empty(){return;}
            let labels=self.ivars().labels.borrow();
            let Some(labels)=labels.as_ref() else {return};
            let selected=view.selectedRange();
            for (row,&start) in rows {
                let (line,baseline)=if start<length {
                    let glyph=layout.glyphIndexForCharacterAtIndex(start);
                    let line=unsafe{layout.lineFragmentRectForGlyphAtIndex_effectiveRange(glyph,std::ptr::null_mut())};
                    (line,line.origin.y+layout.locationForGlyphAtIndex(glyph).y)
                } else {
                    let line=layout.extraLineFragmentRect();
                    if line.size.height<=0. {continue;}
                    (line,line.origin.y+self.ivars().empty_baseline.get())
                };
                let origin=self.convertPoint_fromView(NSPoint::new(0.,line.origin.y+container_origin.y),Some(view));
                let baseline=self.convertPoint_fromView(NSPoint::new(0.,baseline+container_origin.y),Some(view)).y;
                if view.ivars().current_line.get() && selected.length==0 && selected.location>=start && selected.location<starts.get(row+1).copied().unwrap_or(usize::MAX) {
                    let c=view.ivars().line_color.get();NSColor::colorWithSRGBRed_green_blue_alpha(c.r() as f64/255.,c.g() as f64/255.,c.b() as f64/255.,0.5).set();NSRectFill(NSRect::new(NSPoint::new(bounds.origin.x,origin.y),NSSize::new(bounds.size.width,line.size.height)));
                }
                let digits=(row+1).to_string();
                let x=bounds.origin.x+bounds.size.width-self.ivars().padding.get()-labels.advance*digits.len() as f64;
                // Draw cached digit glyphs at the source layout's actual baseline.
                for (column,digit) in digits.bytes().enumerate() {
                    let (range,position)=labels.positions[(digit-b'0') as usize];
                    labels.glyphs.layout.drawGlyphsForGlyphRange_atPoint(range,NSPoint::new(x+column as f64*labels.advance-position.x,baseline-labels.baseline));
                }
            }
        }
    }
    unsafe impl NSObjectProtocol for LineRuler {}
);
// Keep the storage alive while its layout manager measures or draws glyphs.
struct GlyphLayout {
    _storage: Retained<NSTextStorage>,
    layout: Retained<NSLayoutManager>,
}
impl GlyphLayout {
    fn new(text: &str, attributes: &NSDictionary<NSString, AnyObject>) -> Self {
        let storage = NSTextStorage::new();
        storage.replaceCharactersInRange_withString(NSRange::new(0, 0), &NSString::from_str(text));
        unsafe {
            storage.setAttributes_range(Some(attributes), NSRange::new(0, storage.length()));
        }
        let layout = NSLayoutManager::new();
        let container = NSTextContainer::new();
        container.setContainerSize(NSSize::new(1e9, 1e9));
        container.setLineFragmentPadding(0.);
        layout.addTextContainer(&container);
        storage.addLayoutManager(&layout);
        layout.ensureLayoutForTextContainer(&container);
        Self {
            _storage: storage,
            layout,
        }
    }
    fn baseline(&self) -> f64 {
        let glyph = self.layout.glyphIndexForCharacterAtIndex(0);
        let line = unsafe {
            self.layout
                .lineFragmentRectForGlyphAtIndex_effectiveRange(glyph, std::ptr::null_mut())
        };
        line.origin.y + self.layout.locationForGlyphAtIndex(glyph).y
    }
}
// A fixed ten-glyph layout keeps scrolling independent of label typesetting.
struct DigitLabels {
    glyphs: GlyphLayout,
    positions: [(NSRange, NSPoint); 10],
    advance: f64,
    baseline: f64,
}
impl DigitLabels {
    fn new(font: &NSFont) -> Self {
        let color = NSColor::secondaryLabelColor();
        let attributes = NSDictionary::from_slices(
            &[unsafe { NSFontAttributeName }, unsafe {
                NSForegroundColorAttributeName
            }],
            &[font as &AnyObject, &*color as &AnyObject],
        );
        let glyphs = GlyphLayout::new("0123456789", &attributes);
        let positions = std::array::from_fn(|digit| {
            let range = unsafe {
                glyphs
                    .layout
                    .glyphRangeForCharacterRange_actualCharacterRange(
                        NSRange::new(digit, 1),
                        std::ptr::null_mut(),
                    )
            };
            (range, glyphs.layout.locationForGlyphAtIndex(range.location))
        });
        let advance =
            unsafe { NSString::from_str("8").sizeWithAttributes(Some(&attributes)) }.width;
        let baseline = glyphs.baseline();
        Self {
            glyphs,
            positions,
            advance,
            baseline,
        }
    }
}
fn ruler_font(view: &SourceView) -> Retained<NSFont> {
    NSFont::monospacedDigitSystemFontOfSize_weight(
        view.font().map(|font| font.pointSize()).unwrap_or(14.) * 0.86,
        0.,
    )
}
struct Slot {
    scroll: Retained<NSScrollView>,
    text: Retained<SourceView>,
    ruler: Retained<LineRuler>,
    generation: u64,
    style: Option<Style>,
    highlighted: Option<u64>,
    applied: usize,
    selection: Option<NSRange>,
    ruler_digits: usize,
}
#[derive(PartialEq)]
struct Style {
    font: f32,
    line_height: f32,
    indent: usize,
    proportional: bool,
    family: String,
    wrap: bool,
    zoom: f32,
    canvas: egui::Color32,
    text: egui::Color32,
}
pub struct Source<'a> {
    pub id: u64,
    pub text: &'a str,
    pub generation: u64,
    pub metadata: &'a editor::Metadata,
    pub highlight: Option<&'a syntax::Highlight>,
    pub read_only: bool,
}
type MenuItems = Vec<(Action, Retained<NSMenuItem>)>;
type CommandMenus = (Option<Retained<CommandTarget>>, MenuItems);
pub struct Editors {
    slots: HashMap<u64, Slot>,
    locked: HashSet<u64>,
    commands: Rc<RefCell<Vec<Action>>>,
    ctx: egui::Context,
    wake: Arc<dyn Fn() + Send + Sync>,
    shown: Option<u64>,
    bindings: Rc<RefCell<Vec<Binding>>>,
    monitor: Option<Retained<AnyObject>>,
    host: Rc<RefCell<Option<Retained<NSView>>>>,
    menu_items: Vec<(Action, Retained<NSMenuItem>)>,
    _menu_target: Option<Retained<CommandTarget>>,
}
impl Editors {
    pub fn new(ctx: egui::Context, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let commands = Rc::new(RefCell::new(Vec::new()));
        let queue = commands.clone();
        let bindings = Rc::new(RefCell::new(
            commands::bindings(&Default::default()).expect("Default shortcuts are valid"),
        ));
        let keymap = bindings.clone();
        let repaint = ctx.clone();
        let redraw = wake.clone();
        let host: Rc<RefCell<Option<Retained<NSView>>>> = Default::default();
        let command_host = host.clone();
        let block =
            block2::RcBlock::new(move |event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
                let value = unsafe { event.as_ref() };
                // App commands share one native route even when an egui field owns focus.
                // Passing them through Winit and the menu key equivalent can lose a repaint.
                if let Some(action) = event_action(value, &keymap.borrow()) {
                    if matches!(
                        action,
                        Action::Path
                            | Action::Files
                            | Action::Text
                            | Action::Find
                            | Action::Palette
                    ) && let Some(host) = command_host.borrow().clone()
                        && let Some(window) = host.window()
                        && value
                            .window(MainThreadMarker::new().unwrap())
                            .is_some_and(|event_window| std::ptr::eq(&*event_window, &*window))
                    {
                        window.makeFirstResponder(Some(&host));
                    }
                    queue.borrow_mut().push(action);
                    repaint.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
                    repaint.request_repaint_of(egui::ViewportId::ROOT);
                    redraw();
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            });
        let monitor = if std::env::var_os("GLIMPSE_EGUI_EDITOR").is_none() {
            install_edit_menu();
            unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
            }
        } else {
            None
        };
        bindings.borrow_mut().clear();
        let (menu_target, menu_items) = if std::env::var_os("GLIMPSE_EGUI_EDITOR").is_none() {
            install_command_menus(commands.clone(), ctx.clone(), wake.clone())
        } else {
            (None, vec![])
        };
        Self {
            slots: HashMap::new(),
            locked: HashSet::new(),
            commands,
            ctx,
            wake,
            shown: None,
            bindings,
            monitor,
            host,
            menu_items,
            _menu_target: menu_target,
        }
    }

    pub fn set_bindings(&self, bindings: Vec<Binding>) {
        if *self.bindings.borrow() != bindings {
            *self.bindings.borrow_mut() = bindings.clone();
            for (action, item) in &self.menu_items {
                if let Some(binding) = bindings.iter().find(|b| b.action == *action) {
                    item.setKeyEquivalent(&NSString::from_str(&native_key(binding.key)));
                    let mut flags = NSEventModifierFlags::Command;
                    if binding.modifiers.shift {
                        flags |= NSEventModifierFlags::Shift;
                    }
                    if binding.modifiers.alt {
                        flags |= NSEventModifierFlags::Option;
                    }
                    if binding.modifiers.ctrl {
                        flags |= NSEventModifierFlags::Control;
                    }
                    item.setKeyEquivalentModifierMask(flags);
                } else {
                    item.setKeyEquivalent(&NSString::new());
                }
            }
        }
    }
    pub fn drawn(&self, id: u64) -> bool {
        self.slots
            .get(&id)
            .is_some_and(|slot| slot.text.ivars().drawn.get())
    }
    pub fn is_locked(&self, id: u64) -> bool {
        self.locked.contains(&id)
    }
    pub fn set_locked(&mut self, id: u64, locked: bool) {
        if locked {
            self.locked.insert(id);
        } else {
            self.locked.remove(&id);
        }
        if let Some(slot) = self.slots.get(&id) {
            slot.text.setEditable(!locked);
        }
    }
    pub fn focus_host(&self) {
        if let Some(host) = self.host.borrow().clone()
            && let Some(window) = host.window()
        {
            window.makeFirstResponder(Some(&host));
        }
    }
    pub fn set_available(&self, available: &[Action]) {
        for (action, item) in &self.menu_items {
            let enabled = available.contains(action);
            if item.isEnabled() != enabled {
                item.setEnabled(enabled);
            }
        }
    }
    pub fn menu_label(&self, action: Action, label: &str) {
        if let Some((_, item)) = self.menu_items.iter().find(|(a, _)| *a == action) {
            item.setTitle(&NSString::from_str(label));
        }
    }
    pub fn indent(&self, id: u64, outdent: bool) {
        if let Some(slot) = self.slots.get(&id) {
            slot.text.indent(outdent);
        }
    }
    pub fn commands(&self) -> Vec<Action> {
        let commands: Vec<_> = self.commands.borrow_mut().drain(..).collect();
        if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some() && !commands.is_empty() {
            eprintln!("NATIVE_COMMANDS {commands:?}");
        }
        commands
    }
    pub fn changes(&mut self) -> Vec<(u64, String)> {
        let _timing = crate::perf::Span::new("native_changes");
        self.slots
            .iter_mut()
            .filter_map(|(&id, slot)| {
                if slot.text.ivars().dirty.replace(false) {
                    Some((id, snapshot(&slot.text.string())))
                } else {
                    None
                }
            })
            .collect()
    }
    pub fn acknowledge(&mut self, id: u64, generation: u64) {
        if let Some(slot) = self.slots.get_mut(&id) {
            slot.generation = generation;
        }
    }
    pub fn retain(&mut self, ids: &[u64]) {
        if self
            .slots
            .keys()
            .any(|id| !ids.contains(id) && self.focused(*id))
        {
            self.focus_host();
        }
        self.slots.retain(|id, slot| {
            if !ids.contains(id) {
                slot.scroll.removeFromSuperview();
                false
            } else {
                true
            }
        });
    }
    pub fn begin(&mut self) {
        self.shown = None;
    }
    pub fn finish(&self) {
        for (&id, slot) in &self.slots {
            let hide = self.shown != Some(id);
            if slot.scroll.isHidden() != hide {
                let focused = slot.text.window().is_some_and(|window| {
                    window.firstResponder().is_some_and(|responder| {
                        std::ptr::eq(
                            Retained::as_ptr(&responder).cast::<AnyObject>(),
                            Retained::as_ptr(&slot.text).cast::<AnyObject>(),
                        )
                    })
                });
                slot.scroll.setHidden(hide);
                if hide
                    && focused
                    && let Some(host) = unsafe { slot.scroll.superview() }
                    && let Some(window) = host.window()
                {
                    window.makeFirstResponder(Some(&host));
                }
            }
        }
    }
    pub fn composing(&self) -> bool {
        self.slots
            .values()
            .any(|slot| unsafe { msg_send![&*slot.text, hasMarkedText] })
    }
    pub fn focused(&self, id: u64) -> bool {
        self.slots.get(&id).is_some_and(|slot| {
            slot.text
                .window()
                .and_then(|w| w.firstResponder())
                .is_some_and(|r| {
                    std::ptr::eq(
                        (&*r) as *const _ as *const (),
                        (&*slot.text) as *const _ as *const (),
                    )
                })
        })
    }
    pub fn selection(&self, id: u64) -> Option<NSRange> {
        self.slots.get(&id).map(|s| s.text.selectedRange())
    }
    pub fn select(&self, id: u64, start: usize, end: usize) {
        if let Some(slot) = self.slots.get(&id) {
            let range = NSRange::new(start, end.saturating_sub(start));
            slot.text.setSelectedRange(range);
            slot.text.setNeedsDisplay(true);
            slot.ruler.setNeedsDisplay(true);
            unsafe {
                let _: () = msg_send![&*slot.text,scrollRangeToVisible:range];
            }
        }
    }
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        frame: &eframe::Frame,
        source: Source<'_>,
        settings: &Settings,
        focus: bool,
    ) -> Result<(), String> {
        let Source {
            id,
            text,
            generation,
            metadata,
            highlight,
            read_only,
        } = source;
        let read_only = read_only || self.locked.contains(&id);
        let RawWindowHandle::AppKit(handle) =
            frame.window_handle().map_err(|e| e.to_string())?.as_raw()
        else {
            return Err("AppKit window required".into());
        };
        let host = unsafe { &*(handle.ns_view.as_ptr() as *const NSView) };
        if self.host.borrow().is_none() {
            *self.host.borrow_mut() = Some(
                unsafe { Retained::retain(host as *const NSView as *mut NSView) }
                    .ok_or("Native host is unavailable")?,
            );
        }
        let mtm = MainThreadMarker::new().ok_or("Source view requires main thread")?;
        install_accessibility_bridge(host);
        let parent = host;
        self.shown = Some(id);
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, egui::Sense::hover());
        let zoom = ui.ctx().zoom_factor() as f64;
        let bounds = parent.bounds();
        let native_rect = NSRect::new(
            NSPoint::new(
                rect.left() as f64 * zoom,
                if parent.isFlipped() {
                    rect.top() as f64 * zoom
                } else {
                    bounds.size.height - rect.bottom() as f64 * zoom
                },
            ),
            NSSize::new(rect.width() as f64 * zoom, rect.height() as f64 * zoom),
        );
        let slot = self.slots.entry(id).or_insert_with(|| {
            let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), native_rect);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            let allocated = SourceView::alloc(mtm).set_ivars(Ivars {
                current_line: Cell::new(settings.current_line),
                long_line: Cell::new(metadata.longest_line > 16384),
                line_color: Cell::new(appearance::palette(ui).selection),
                dirty: Cell::new(false),
                drawn:Cell::new(false),
                suppress: Cell::new(false),
                ctx: self.ctx.clone(),
                wake: self.wake.clone(),
                indent: Cell::new(settings.indent),
                tabs: Cell::new(settings.tabs),
                auto_indent: Cell::new(settings.auto_indent),
                bindings: self.bindings.clone(),
                queue:self.commands.clone(),
            });
            let view: Retained<SourceView> =
                unsafe { msg_send![super(allocated),initWithFrame:NSRect::new(NSPoint::new(0.0,0.0),native_rect.size)] };
            if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some(){eprintln!("NATIVE_CLASS {:?} expected {:?}",view.class().name(),SourceView::class().name());}
            assert!(view.isKindOfClass(SourceView::class()),"AppKit replaced source subclass during initialization");
            view.setRichText(false);
            view.setImportsGraphics(false);
            view.setAllowsUndo(true);
            view.setUsesFindPanel(false);
            view.setAutomaticQuoteSubstitutionEnabled(false);
            view.setAutomaticDashSubstitutionEnabled(false);
            view.setAutomaticTextReplacementEnabled(false);
            view.setAutomaticSpellingCorrectionEnabled(false);
            view.setContinuousSpellCheckingEnabled(false);
            view.setMinSize(NSSize::new(0., 0.));
            view.setMaxSize(NSSize::new(1e9, 1e9));
            view.setVerticallyResizable(true);
            view.setTextContainerInset(NSSize::new(8.0, 6.0));
            unsafe {
                if let Some(layout) = view.layoutManager() {
                    layout.setAllowsNonContiguousLayout(true);
                    layout.setBackgroundLayoutEnabled(false);
                }
            }
            if metadata.longest_line > 16384 {
                view.setHorizontallyResizable(true);
                if let Some(container) = unsafe { view.textContainer() } {
                    container.setWidthTracksTextView(false);
                    container.setContainerSize(NSSize::new(1e9,1e9));
                }
            }
            view.setString(&NSString::from_str(text));
            // New documents start at the first character; CLI positions apply afterwards.
            view.setSelectedRange(NSRange::new(0, 0));
            scroll.setDocumentView(Some(&view));
            let ruler=unsafe{msg_send![super(LineRuler::alloc(mtm).set_ivars(RulerIvars{text:view.clone(),starts:RefCell::new(metadata.utf16_starts.clone()),empty_baseline:Cell::new(0.),labels:RefCell::new(None),padding:Cell::new(12.)})),initWithScrollView:&*scroll,orientation:NSRulerOrientation::VerticalRuler]};
            let ruler:Retained<LineRuler>=ruler;
            ruler.setRuleThickness(40.);
            scroll.setVerticalRulerView(Some(&ruler));scroll.setHasVerticalRuler(true);scroll.setRulersVisible(true);

            parent.addSubview(&scroll);
            Slot {
                scroll,
                ruler,
                text: view,
                generation, style: None,highlighted: None,applied:0,selection:None,ruler_digits:0,
            }
        });
        if slot.scroll.frame() != native_rect {
            slot.scroll.setFrame(native_rect);
        }
        let selection = slot.text.selectedRange();
        if slot.selection != Some(selection) {
            slot.selection = Some(selection);
            slot.text.setNeedsDisplay(true);
            slot.ruler.setNeedsDisplay(true);
        }
        if slot.scroll.isHidden() {
            slot.text.ivars().drawn.set(false);
            slot.text.setNeedsDisplay(true);
            slot.scroll.setHidden(false);
        }
        if !Arc::ptr_eq(&slot.ruler.ivars().starts.borrow(), &metadata.utf16_starts) {
            *slot.ruler.ivars().starts.borrow_mut() = metadata.utf16_starts.clone();
            slot.ruler.setNeedsDisplay(true);
        }
        if slot.text.isEditable() == read_only {
            slot.text.setEditable(!read_only);
        }
        let long_line = metadata.longest_line > 16384;
        let wrap = settings.wrap && !long_line;
        if slot
            .text
            .ivars()
            .current_line
            .replace(settings.current_line)
            != settings.current_line
        {
            slot.text.setNeedsDisplay(true);
        }
        slot.text.ivars().long_line.set(long_line);
        slot.text
            .ivars()
            .line_color
            .set(appearance::palette(ui).selection);
        slot.text.ivars().indent.set(settings.indent);
        slot.text.ivars().tabs.set(settings.tabs);
        slot.text.ivars().auto_indent.set(settings.auto_indent);
        if slot.scroll.rulersVisible() != settings.line_numbers {
            slot.scroll.setRulersVisible(settings.line_numbers);
        }
        if slot.generation != generation {
            let existing = slot.text.string();
            let value = NSString::from_str(text);
            if existing != value {
                slot.text.ivars().suppress.set(true);
                let old = existing.to_string();
                let (removed, inserted) = editor::difference(&old, text);
                let start = editor::byte_to_utf16(&old, removed.start);
                let end = editor::byte_to_utf16(&old, removed.end);
                let replacement = NSString::from_str(&text[inserted]);
                let range = NSRange::new(start, end - start);
                let selected = slot.text.selectedRange();
                let new_end = start + replacement.length();
                let remap = |index: usize| {
                    if index <= start {
                        index
                    } else if index >= end {
                        new_end + index - end
                    } else {
                        new_end
                    }
                };
                if read_only {
                    slot.text.setString(&value);
                } else if slot
                    .text
                    .shouldChangeTextInRange_replacementString(range, Some(&replacement))
                {
                    unsafe {
                        let _: () = msg_send![&*slot.text,replaceCharactersInRange:range,withString:&*replacement];
                    }
                    slot.text.didChangeText();
                    slot.text.setSelectedRange(NSRange::new(
                        remap(selected.location),
                        remap(selected.location + selected.length)
                            .saturating_sub(remap(selected.location)),
                    ));
                }
                slot.text.ivars().suppress.set(false);
            }
            slot.generation = generation;
        }
        let palette = appearance::palette(ui);
        let style = Style {
            font: settings.font_size,
            line_height: settings.line_height,
            indent: settings.indent,
            proportional: settings.proportional,
            family: settings.font_family.clone(),
            wrap,
            zoom: zoom as f32,
            canvas: palette.canvas,
            text: palette.text,
        };
        if slot.style.as_ref() != Some(&style) {
            let font = if !settings.font_family.is_empty() {
                NSFont::fontWithName_size(
                    &NSString::from_str(&settings.font_family),
                    settings.font_size as f64 * zoom,
                )
                .ok_or_else(|| format!("Font unavailable: {}", settings.font_family))?
            } else if settings.proportional {
                NSFont::systemFontOfSize(settings.font_size as f64 * zoom)
            } else {
                NSFont::monospacedSystemFontOfSize_weight(settings.font_size as f64 * zoom, 0.0)
            };
            slot.text.setFont(Some(&font));
            slot.text
                .setTextContainerInset(NSSize::new(8. * zoom, 6. * zoom));
            // Set layout attributes without replacing characters, selection or native undo.
            let paragraph = NSMutableParagraphStyle::new();
            paragraph.setMinimumLineHeight(
                settings.font_size as f64 * settings.line_height as f64 * zoom,
            );
            let font_attributes = NSDictionary::from_slices(
                &[unsafe { NSFontAttributeName }],
                &[&*font as &AnyObject],
            );
            let space =
                unsafe { NSString::from_str(" ").sizeWithAttributes(Some(&font_attributes)) }.width;
            // AppKit's explicit default stops otherwise override the repeating interval.
            paragraph.setTabStops(Some(&NSArray::new()));
            paragraph.setDefaultTabInterval(space * settings.indent.max(1) as f64);
            slot.text.setDefaultParagraphStyle(Some(&paragraph));
            let typing = NSDictionary::from_slices(
                &[unsafe { NSFontAttributeName }, unsafe {
                    NSParagraphStyleAttributeName
                }],
                &[&*font as &AnyObject, &*paragraph as &AnyObject],
            );
            slot.ruler
                .ivars()
                .empty_baseline
                .set(GlyphLayout::new(" ", &typing).baseline());
            slot.ruler.ivars().padding.set(12. * zoom);
            *slot.ruler.ivars().labels.borrow_mut() =
                Some(DigitLabels::new(&ruler_font(&slot.text)));
            slot.ruler_digits = 0;
            if let Some(storage) = unsafe { slot.text.textStorage() } {
                slot.text.ivars().suppress.set(true);
                unsafe {
                    storage.addAttribute_value_range(
                        NSParagraphStyleAttributeName,
                        &paragraph,
                        NSRange::new(0, slot.text.string().length()),
                    );
                }
                slot.text.ivars().suppress.set(false);
            }

            let color = |c: egui::Color32| {
                NSColor::colorWithSRGBRed_green_blue_alpha(
                    c.r() as f64 / 255.0,
                    c.g() as f64 / 255.0,
                    c.b() as f64 / 255.0,
                    1.0,
                )
            };
            slot.text.setBackgroundColor(&color(palette.canvas));
            let text_color = color(palette.text);
            slot.text.setTextColor(Some(&text_color));
            let typing = NSDictionary::from_slices(
                &[
                    unsafe { NSFontAttributeName },
                    unsafe { NSParagraphStyleAttributeName },
                    unsafe { NSForegroundColorAttributeName },
                ],
                &[
                    &*font as &AnyObject,
                    &*paragraph as &AnyObject,
                    &*text_color as &AnyObject,
                ],
            );
            unsafe {
                slot.text.setTypingAttributes(&typing);
            }
            slot.text
                .setInsertionPointColor(Some(&color(palette.accent)));
            slot.scroll.setBackgroundColor(&color(palette.canvas));
            slot.scroll.setHasHorizontalScroller(!wrap);
            slot.text.setHorizontallyResizable(!wrap);
            slot.text.setAutoresizingMask(if wrap {
                NSAutoresizingMaskOptions::ViewWidthSizable
            } else {
                NSAutoresizingMaskOptions::empty()
            });
            unsafe {
                if let Some(container) = slot.text.textContainer() {
                    container.setWidthTracksTextView(false);
                    if !wrap {
                        container.setContainerSize(NSSize::new(1e9, 1e9));
                    }
                }
            }
            if std::env::var_os("GLIMPSE_NATIVE_TRACE").is_some() {
                eprintln!(
                    "NATIVE_LAYOUT wrap={} clip={:?} view={:?} container={:?}",
                    wrap,
                    slot.scroll.contentSize(),
                    slot.text.frame(),
                    unsafe { slot.text.textContainer() }.map(|c| c.containerSize())
                );
            }
            slot.ruler.setNeedsDisplay(true);
            slot.style = Some(style);
            slot.highlighted = None;
        }
        let digits = metadata.starts.len().max(1).ilog10() as usize + 1;
        if slot.ruler_digits != digits {
            let number_width = slot
                .ruler
                .ivars()
                .labels
                .borrow()
                .as_ref()
                .map(|labels| labels.advance * digits as f64)
                .unwrap_or(0.);
            let width = (number_width + 2. * slot.ruler.ivars().padding.get()).max(40. * zoom);
            if slot.ruler.ruleThickness() != width {
                slot.ruler.setRuleThickness(width);
            }
            slot.ruler_digits = digits;
        }
        if wrap && let Some(container) = unsafe { slot.text.textContainer() } {
            // AppKit includes the ruler's negative-origin inset in contentSize; reserve its visible width.
            let width = (slot.scroll.contentSize().width
                - if slot.scroll.rulersVisible() {
                    slot.ruler.ruleThickness()
                } else {
                    0.
                })
            .max(1.);
            let frame = slot.text.frame();
            if frame.size.width != width {
                slot.text
                    .setFrameSize(NSSize::new(width, frame.size.height));
            }
            let size = NSSize::new(
                (width - 2. * slot.text.textContainerInset().width).max(1.),
                1e9,
            );
            if container.containerSize() != size {
                container.setContainerSize(size);
            }
        }
        if !settings.syntax
            && slot.highlighted.take().is_some()
            && let Some(layout) = unsafe { slot.text.layoutManager() }
        {
            layout.removeTemporaryAttribute_forCharacterRange(
                unsafe { NSForegroundColorAttributeName },
                NSRange::new(0, slot.text.string().length()),
            );
            layout.removeTemporaryAttribute_forCharacterRange(
                unsafe { NSFontAttributeName },
                NSRange::new(0, slot.text.string().length()),
            );
        }
        if settings.syntax
            && let Some(highlight) = highlight
            && let Some(layout) = unsafe { slot.text.layoutManager() }
        {
            if slot.highlighted != Some(highlight.revision) {
                layout.removeTemporaryAttribute_forCharacterRange(
                    unsafe { NSForegroundColorAttributeName },
                    NSRange::new(0, slot.text.string().length()),
                );
                layout.removeTemporaryAttribute_forCharacterRange(
                    unsafe { NSFontAttributeName },
                    NSRange::new(0, slot.text.string().length()),
                );
                slot.highlighted = Some(highlight.revision);
                slot.applied = 0;
            }
            let end = (slot.applied + 256).min(highlight.spans.len());
            let italic_font = if highlight.spans[slot.applied..end]
                .iter()
                .any(|span| span.italic)
            {
                slot.text.font().and_then(|font| {
                    let descriptor = font.fontDescriptor();
                    NSFont::fontWithDescriptor_size(
                        &descriptor.fontDescriptorWithSymbolicTraits(
                            descriptor.symbolicTraits()
                                | NSFontDescriptorSymbolicTraits::TraitItalic,
                        ),
                        font.pointSize(),
                    )
                })
            } else {
                None
            };
            for span in &highlight.spans[slot.applied..end] {
                let rgb = syntax::readable_rgb(
                    span.rgb,
                    [palette.canvas.r(), palette.canvas.g(), palette.canvas.b()],
                );
                let color = NSColor::colorWithSRGBRed_green_blue_alpha(
                    rgb[0] as f64 / 255.,
                    rgb[1] as f64 / 255.,
                    rgb[2] as f64 / 255.,
                    1.,
                );
                if span.italic
                    && let Some(font) = &italic_font
                {
                    unsafe {
                        layout.addTemporaryAttributes_forCharacterRange(
                            &NSDictionary::from_slices(
                                &[NSFontAttributeName],
                                &[&**font as &AnyObject],
                            ),
                            NSRange::new(span.start, span.length),
                        );
                    }
                }
                let attributes = NSDictionary::from_slices(
                    &[unsafe { NSForegroundColorAttributeName }],
                    &[&*color as &AnyObject],
                );
                unsafe {
                    layout.addTemporaryAttributes_forCharacterRange(
                        &attributes,
                        NSRange::new(span.start, span.length),
                    );
                }
            }
            slot.applied = end;
            if end < highlight.spans.len() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
                ui.ctx().request_repaint();
            }
        }
        if focus && let Some(window) = host.window() {
            window.makeFirstResponder(Some(&slot.text));
        }
        Ok(())
    }
}
impl Drop for Editors {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            unsafe {
                NSEvent::removeMonitor(&monitor);
            }
        }
        for slot in self.slots.values() {
            slot.scroll.removeFromSuperview();
        }
    }
}

pub fn valid_font(name: &str) -> bool {
    name.is_empty() || NSFont::fontWithName_size(&NSString::from_str(name), 14.).is_some()
}
fn event_action(event: &NSEvent, bindings: &[Binding]) -> Option<Action> {
    let text = event.charactersIgnoringModifiers()?.to_string();
    let character = text.chars().next()?;
    let flags = event.modifierFlags();
    if character == '\u{1b}'
        && !flags.intersects(
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Shift
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Control,
        )
    {
        let window = event.window(MainThreadMarker::new()?)?;
        if AnyClass::get(c"NSPanel").is_some_and(|class| window.isKindOfClass(class)) {
            return None;
        }
        if let Some(responder) = window.firstResponder()
            && responder.respondsToSelector(sel!(hasMarkedText))
            && unsafe { msg_send![&*responder, hasMarkedText] }
        {
            return None;
        }
        // Keep dismissal ordered with subsequent app commands instead of Winit events.
        return Some(Action::Cancel);
    }
    let key = match character {
        '\u{f700}' => egui::Key::ArrowUp,
        '\u{f701}' => egui::Key::ArrowDown,
        '\u{f702}' => egui::Key::ArrowLeft,
        '\u{f703}' => egui::Key::ArrowRight,
        '\r' => egui::Key::Enter,
        ',' => egui::Key::Comma,
        c => egui::Key::from_name(&c.to_uppercase().to_string())?,
    };
    bindings
        .iter()
        .find(|b| {
            b.key == key
                && b.modifiers.command == flags.contains(NSEventModifierFlags::Command)
                && b.modifiers.shift == flags.contains(NSEventModifierFlags::Shift)
                && b.modifiers.alt == flags.contains(NSEventModifierFlags::Option)
                && b.modifiers.ctrl == flags.contains(NSEventModifierFlags::Control)
        })
        .map(|b| b.action)
}
type Children = unsafe extern "C" fn(&NSView, Sel) -> *mut NSArray<NSObject>;
static PREVIOUS_CHILDREN: std::sync::OnceLock<Children> = std::sync::OnceLock::new();
unsafe extern "C" fn merged_children(view: &NSView, selector: Sel) -> *mut NSArray<NSObject> {
    let original = unsafe { PREVIOUS_CHILDREN.get().unwrap()(view, selector) };
    let mut children = if original.is_null() {
        Vec::new()
    } else {
        unsafe { &*original }.to_vec()
    };
    for child in view.subviews().to_vec() {
        if !child.isHidden() && child.isKindOfClass(NSScrollView::class()) {
            children.push(unsafe { Retained::cast_unchecked::<NSObject>(child) });
        }
    }
    Retained::autorelease_ptr(NSArray::from_retained_slice(&children))
}
fn install_accessibility_bridge(view: &NSView) {
    if PREVIOUS_CHILDREN.get().is_some() {
        return;
    }
    let class = view.class();
    let selector = sel!(accessibilityChildren);
    let Some(method) = class.instance_method(selector) else {
        return;
    };
    // Keep Winit's content view in place: its delegate casts it to WinitView.
    // AccessKit's root children omit native subviews, so append the source scroll area.
    unsafe {
        let previous = std::mem::transmute::<Imp, Children>(method.implementation());
        if PREVIOUS_CHILDREN.set(previous).is_err() {
            return;
        }
        let implementation = std::mem::transmute::<Children, Imp>(merged_children as Children);
        let encoding = objc2::ffi::method_getTypeEncoding(method);
        if !objc2::ffi::class_addMethod(
            class as *const AnyClass as *mut AnyClass,
            selector,
            implementation,
            encoding,
        )
        .as_bool()
        {
            method.set_implementation(implementation);
        }
    }
}

fn native_key(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowUp => "\u{f700}".into(),
        egui::Key::ArrowDown => "\u{f701}".into(),
        egui::Key::ArrowLeft => "\u{f702}".into(),
        egui::Key::ArrowRight => "\u{f703}".into(),
        egui::Key::Enter => "\r".into(),
        egui::Key::Comma => ",".into(),
        egui::Key::Space => " ".into(),
        egui::Key::Home => "\u{f729}".into(),
        egui::Key::End => "\u{f72b}".into(),
        egui::Key::PageUp => "\u{f72c}".into(),
        egui::Key::PageDown => "\u{f72d}".into(),
        egui::Key::Delete => "\u{f728}".into(),
        egui::Key::Backspace => "\u{8}".into(),
        egui::Key::Escape => "\u{1b}".into(),
        egui::Key::Tab => "\t".into(),
        key => {
            let name = key.name();
            if let Some(n) = name.strip_prefix('F').and_then(|s| s.parse::<u32>().ok()) {
                char::from_u32(0xf703 + n).unwrap().to_string()
            } else {
                key.symbol_or_name().to_lowercase()
            }
        }
    }
}
fn install_command_menus(
    queue: Rc<RefCell<Vec<Action>>>,
    ctx: egui::Context,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> CommandMenus {
    let Some(mtm) = MainThreadMarker::new() else {
        return (None, vec![]);
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(main) = app.mainMenu() else {
        return (None, vec![]);
    };
    if let Some(menu) = main.itemAtIndex(0).and_then(|i| i.submenu()) {
        for item in menu.itemArray().to_vec() {
            if item.action() == Some(sel!(terminate:)) {
                item.setKeyEquivalent(&NSString::new());
            }
        }
    }
    let target: Retained<CommandTarget> = unsafe {
        msg_send![
            super(CommandTarget::alloc(mtm).set_ivars(CommandIvars { queue, ctx, wake })),
            init
        ]
    };
    let mut items = vec![];
    for category in ["File", "Navigate", "Search", "View", "Request"] {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(category));
        menu.setAutoenablesItems(false);
        for (index, command) in commands::COMMANDS.iter().enumerate() {
            use Action::*;
            let group = match command.action {
                New | Open | Folder | Save | SaveAs | SaveCopy | Close | Quit | NewFolder
                | NamedFile | Rename | Trash => "File",
                Back | Forward | Home | Parent | Refresh | Path | Pin | Scope | GoScope
                | CopyPath | CopyRelative | Reveal | External | Finder => "Navigate",
                Files | Text | Find => "Search",
                Finish | Abort => "Request",
                _ => "View",
            };
            if group != category {
                continue;
            }
            let item = unsafe {
                menu.addItemWithTitle_action_keyEquivalent(
                    &NSString::from_str(command.label),
                    Some(sel!(invoke:)),
                    &NSString::new(),
                )
            };
            item.setTag(index as isize);
            unsafe {
                item.setTarget(Some(&target));
            }
            items.push((command.action, item));
        }
        let parent = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(category),
                None,
                &NSString::new(),
            )
        };
        parent.setSubmenu(Some(&menu));
        main.addItem(&parent);
    }
    (Some(target), items)
}
fn install_edit_menu() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let application = NSApplication::sharedApplication(mtm);
    let Some(main) = application.mainMenu() else {
        return;
    };
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit"));
    for (title, key, action, shift) in [
        ("Undo", "z", sel!(undo:), false),
        ("Redo", "z", sel!(redo:), true),
        ("Cut", "x", sel!(cut:), false),
        ("Copy", "c", sel!(copy:), false),
        ("Paste", "v", sel!(paste:), false),
        ("Select All", "a", sel!(selectAll:), false),
    ] {
        let item = unsafe {
            menu.addItemWithTitle_action_keyEquivalent(
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        item.setKeyEquivalentModifierMask(if shift {
            NSEventModifierFlags::Command | NSEventModifierFlags::Shift
        } else {
            NSEventModifierFlags::Command
        });
    }
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Edit"),
            None,
            &NSString::from_str(""),
        )
    };
    item.setSubmenu(Some(&menu));
    main.addItem(&item);
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn native_snapshot_preserves_unicode_nul_and_line_endings() {
        for text in ["", "ascii\0tail\r\n", "α é 👨‍👩‍👧 日本語\rnext\n"] {
            let storage = NSTextStorage::new();
            storage
                .replaceCharactersInRange_withString(NSRange::new(0, 0), &NSString::from_str(text));
            assert_eq!(snapshot(&storage.string()), text);
        }
    }
    #[test]
    #[ignore = "release native string CPU benchmark; excludes input and presentation"]
    fn benchmark_native_snapshot() {
        let mut text = "0123456789 abcdefghijklmnopqrstuvwxyz fixture line\n".repeat(214_000);
        text.truncate(10 * 1024 * 1024);
        let storage = NSTextStorage::new();
        storage.replaceCharactersInRange_withString(NSRange::new(0, 0), &NSString::from_str(&text));
        for name in ["display", "snapshot"] {
            let mut samples = Vec::new();
            for _ in 0..20 {
                let started = std::time::Instant::now();
                let result = if name == "display" {
                    storage.string().to_string()
                } else {
                    snapshot(&storage.string())
                };
                samples.push(started.elapsed().as_secs_f64() * 1000.);
                assert_eq!(result, text);
                std::hint::black_box(result);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "native string {} bytes {name}: p50={:.3}ms p95={:.3}ms max={:.3}ms",
                text.len(),
                samples[10],
                samples[18],
                samples[19]
            );
        }
    }
}
