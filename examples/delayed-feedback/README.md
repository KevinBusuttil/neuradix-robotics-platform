# Offline delayed feedback

`deployment.yaml` declares a one-tick controller/plant loop using standard
contracts. It is an offline topology example, not a running controller or plant.

```sh
timeout 30s cargo run --locked -p neuradix-graph --example delayed_feedback
timeout 30s cargo run --locked -p neuradix-cli -- graph validate examples/delayed-feedback/deployment.yaml --contracts contracts/standard
```

The graph example checks the valid loop and rejects its instantaneous variant.
Future execution must supply trusted seed history before tick zero and enforce
a common logical evaluation schedule. No buffers, clock synchronization or
actuator permissions are supplied. Positive delays select v3 deployment identities;
all-instantaneous graphs retain historical v2 pins. See the
[A08 policy and evidence](../../docs/implementation/WP-A08-Delayed-Feedback-Validation.md).
