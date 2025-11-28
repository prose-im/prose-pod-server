# Detect media type from magic bytes

Trusting the user for providing the media type is an open door for exploits.
In most binary file formats, file contents start with “magic bytes” which can
be used to detect what the file contains. This crate aims to detect a very
small subset of file type, just the ones we need at [Prose].

For more general-purpose crates supporting more extensions, see:

- [`infer`](https://crates.io/crates/infer)
- [`file-format`](https://crates.io/crates/file-format)
- [`tree_magic`](https://crates.io/crates/tree_magic)
- [`file_type`](https://crates.io/crates/file_type)

[Prose]: https://prose.org/ "Prose homepage"
