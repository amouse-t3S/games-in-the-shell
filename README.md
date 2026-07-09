# games-in-the-shell
ASCII art games run on shell.


### development
```
cargo run
```



### setup for building wasi
```
rustup target add wasm32-wasi
```

### build wasm package
```
wasm-pack build --target web
```