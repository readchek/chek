//! Extension host for Chek's remote sources.
//!
//! Sources are WebAssembly components that users install from extension
//! repositories. This crate will run them in a sandbox, list their remote
//! items and fetch and cache their pages. It is empty in the first milestone,
//! which only reads folders; it exists so the catalog and server are shaped
//! around a second kind of library from the start.
