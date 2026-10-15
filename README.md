# Youarel

A launcher that opens your web shortcuts in any browser, from a terminal or the command line.

## Installation

```sh
git clone https://github.com/P4suta/youarel.git
cd youarel
cargo install --locked --path crates/youarel-cli
```

## Usage

```sh
youarel import bookmarks.html
youarel add https://github.com/ --alias gh --folder Work
youarel open gh
youarel
```

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [Massachusetts Institute of Technology (MIT) license](LICENSE-MIT), at your option.
You license each contribution you submit under the same two licenses, without other terms, unless you state otherwise.
