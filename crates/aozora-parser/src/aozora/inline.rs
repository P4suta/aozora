//! Inline dispatch hook.
//!
//! Called from a single line added to `upstream/comrak/src/parser/inline.rs`. The hook
//! receives the current inline cursor and the text already emitted, and decides
//! whether the next bytes start a ruby, bouten, tate-chu-yoko, or gaiji span.

// M0 Spike: module exists so the hook is ready for wiring once upstream vendor lands.
