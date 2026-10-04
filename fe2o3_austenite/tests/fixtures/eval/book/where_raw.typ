// oracle: levels 2 4
#set page(width: 220pt, height: 260pt, margin: 20pt)
#show raw.where(block: true): it => [BR #it.text]
#show raw.where(block: false): it => [IR #it.text]
```rust
fn a() {}
```
`inline`
