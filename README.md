# Hilbert Curve Animation Generator
A small project to generate animations based on Hilbert curves. It can currently generate animations in `gif`, `webp`, and `webm` formats. To add a custom function, write a function with the signature `fn(i: u64, size: u64) -> palette::Srgb<u8>` and add it to the function match statement in `main()`. Have a look at the existing functions for some examples. If you come up with something interesting, please do submit a PR!

# Usage:
```bash
git clone https://github.com/dacid44/hilbert_animation
cd hilbert_animation
cargo run --release -- [OPTIONS] # I HIGHLY recommend running with --release!
```

```
> cargo run -- --help
Usage: hilbert_animation [--order=ARG] [-f=ARG] [-f=ARG] [-r=ARG] [-l=ARG] [-b=ARG] [ARG]

Available options:
        --order=ARG
    -f, --function=ARG
    -f, --frames=ARG
    -r, --framerate=ARG
    -l, --loops=ARG
    -b, --bitrate=ARG
    -h, --help           Prints help information
```

