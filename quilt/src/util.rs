pub fn bx<T>(x: T) -> Box<T> {
    Box::new(x)
}

pub fn rc<T>(x: T) -> std::rc::Rc<T> {
    std::rc::Rc::new(x)
}

pub fn arc<T>(x: T) -> std::sync::Arc<T> {
    std::sync::Arc::new(x)
}

pub const SEP: &str = "/**************************************************************/";
pub fn sep() {
    println!("{SEP}");
}

pub type Index = u8;

/// A byte range into the original `.quilt` source, for diagnostics.
pub type Span = std::ops::Range<usize>;
