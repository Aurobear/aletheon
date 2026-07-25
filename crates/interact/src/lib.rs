//! User interaction: TUI and ACIX.

#![allow(
    clippy::if_same_then_else,
    clippy::unnecessary_sort_by,
    clippy::manual_clamp,
    clippy::ptr_arg,
    clippy::new_without_default,
    clippy::vec_init_then_push,
    clippy::empty_line_after_doc_comments,
    clippy::explicit_counter_loop,
    clippy::too_many_arguments,
    clippy::wrong_self_convention,
    clippy::overly_complex_bool_expr,
    clippy::module_inception,
    private_interfaces,
    unexpected_cfgs,
    deprecated
)]

pub mod acix;
pub mod acp;
pub mod host;
pub mod tui;

/// Backward compatibility: cli module is now tui::cli
pub use tui::cli;
