/// A workaround for a clippy warning
#[doc(hidden)]
#[derive(Clone)]
pub struct Span;

/// A macro to create a span.
#[macro_export]
macro_rules! span {
    (target: $target:expr, $lvl:expr, $name:expr, $($fields:tt)*) => {
        $crate::Span
    };
    (target: $target:expr, $lvl:expr, $name:expr) => {
        $crate::span!(target: $target, $lvl, $name,)
    };
    ($lvl:expr, $name:expr, $($fields:tt)*) => {
        $crate::span!(
            target: module_path!(),
            $lvl,
            $name,
            $($fields)*
        )
    };
    ($lvl:expr, $name:expr) => {
        $crate::span!(
            target: module_path!(),
            $lvl,
            $name,
        )
    };
}
