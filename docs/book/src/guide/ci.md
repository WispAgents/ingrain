# CI

```yaml
# .github/workflows/ingrain.yml
on: pull_request
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: WispAgents/ingrain/action@v0.3.0
```

The action runs `ingrain check --diff base...HEAD --format github`: hits become annotations on the
pull request and a failing check. See [where checks run](checks.md) for how the base revision is used.
