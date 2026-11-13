# peakmark-probe

An opt-in allocator wrapper for a Rust WebAssembly module. It counts the bytes the
module requests and has not freed, the peak of that, and the number of allocations and
frees, and exports them so that `peakmark` can report them. It tracks nothing else.

```rust
use std::alloc::System;
use peakmark_probe::Probe;

#[global_allocator]
static ALLOCATOR: Probe<System> = Probe::new(System);
```

Details: <https://casoon.github.io/peakmark/docs/guides/allocator-probe/>
