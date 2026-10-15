//! The HTML youarel reads and writes: the bookmark files browsers export and import, and the titles and icons of web pages.
//!
//! Both read untrusted markup with one tolerant scan, which never stops on what it does not understand.

pub mod bookmarks;
pub mod icon;
pub mod scan;
pub mod title;
