<!--
Copyright 2026 Ramus

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

> :heart: Thanks for your interest!

## Initial Setup

You will need:

- [Rust] of version `1.88` or higher.
- [Just] of version `1.27` or higher.

1. [Fork] the repository.

2. Clone the fork:

```bash
$ git clone https://github.com/<username>/ramus
```

3. Create your own branch:

```bash
$ git switch -c <name>
```

4. Build and test:

```bash
$ just build
$ just test
```

> See more recipes with `just --list`.

## Before Pushing

Run before every push:

```bash
$ just push
```

Commits must follow [Conventional Commits].

## Adding Predictor

> Predictors live in the `arbiter/src/predictors/` directory.

**1. Write it.** Create `arbiter/src/predictors/<name>.rs` file and implement
the `Predictor` trait. Use [`SmithBimodal`] as the reference.

**2. Register it.** Go to `arbiter/src/predictors.rs` and add one line to the
`predictors!` macro:

```rust
predictors! {
    ...
    "my_predictor" => my_predictor::MyPredictor,
    ...
}
```

**3. Run it.** Download any [CBP] trace, and run:

```bash
$ just run path/to/trace.gz my_predictor
```

**4. Compare it.** Run the following command to compare with [`SmithBimodal`]:

```bash
$ just run path/to/trace.gz my_predictor smith_bimodal
```

<!-- Links -->

[Just]: https://github.com/casey/just
[Fork]: https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/working-with-forks/fork-a-repo
[Conventional Commits]: https://www.conventionalcommits.org/en/v1.0.0/
[`arbiter`]: ./arbiter/src/predictors
[`SmithBimodal`]: ./arbiter/src/predictors/smith_bimodal.rs
[CBP]: https://cbp-ng.bpchamp.com/model#h.kmpqzwlahzq
