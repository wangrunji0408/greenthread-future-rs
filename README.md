# core-thread-future

Convert sync functions into non-blocking thread futures **on bare-metal (no_std)**.

In a word, this is a `#![no_std]` version of [Futurify](https://github.com/robertohuertasm/futurify).

I'm exploring to use it to implement bare-metal threading.

## Example

TODO. 

Now just take a unit test as an example:

```rust
#[tokio::test]
async fn test() {
  let h1 = tokio::spawn(ThreadFuture::new(|| {
    println!("1.1");
    yield_now();
    println!("1.2");
    1u32
  }));
  let h2 = tokio::spawn(ThreadFuture::new(|| {
    println!("2.1");
    yield_now();
    println!("2.2");
    2u32
  }));
  println!("join 1 => {}", h1.await.unwrap());
  println!("join 2 => {}", h2.await.unwrap());
}
```

Output:

```
1.1
2.1
1.2
2.2
join 1 => 1
join 2 => 2
```



