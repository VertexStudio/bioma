# Bioma LLM  

## Running stress tests  

This test is intented to test the performance of the endpoints in the rag server under stress conditions. With one command you can decide which endpoints you want to test.  

```rust
cargo run -p bioma_llm --example attack -- --endpoints <endpoints_list> --order
```  

```endpoints_list``` is a string with the endpoints you want to test separated by commas. The ```--order``` flag is optional and if you use it the endpoints will be tested in the order you passed them. For example:  

```rust
cargo run -p bioma_llm --example attack -- --endpoints health,hello,upload --order
```

will test, for every iteration, the health, hello and upload endpoints in this order.

Available endpoints:

- health
- hello
- index
- chat
- upload
- deletesource
- embed
- ask
- retrieve
- rerank
- all
