# aozora-wasm

A wasm-bindgen driver for the aozora parser. It powers the browser
[playground](https://p4suta.github.io/aozora/playground/).

Install from npm:

```sh
npm install aozora-wasm
```

The renderer emits semantic `aozora-*` class hooks. Applications that want
the project defaults can opt in to the reference stylesheet:

```js
import "aozora-wasm/aozora-notation.css";
```

The stylesheet is optional. Applications can use the class hooks with their
own presentation instead.

Or build from source:

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build --target web --release crates/aozora-wasm
```

The bindings take UTF-8. Real 青空文庫 archive files are Shift_JIS, so
decode host-side before constructing a document.

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
