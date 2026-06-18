# Monitoring

This project exposes Prometheus-compatible metrics so operators can monitor the
health of every union-bridge-client service. Each binary installs a global
`metrics` recorder (the [`metrics`](https://docs.rs/metrics) facade) and serves
`GET /metrics` on its configured bind address. The exporter is
[`metrics-exporter-prometheus`](https://docs.rs/metrics-exporter-prometheus).

The deployment of Prometheus and Grafana is left to each operator; this document
only covers what the services expose and how to scrape them.

## Default endpoints

| Service        | Bind address (base.toml) | Config key                                  |
|----------------|--------------------------|---------------------------------------------|
| coordinator    | `0.0.0.0:9201`           | `coordinator.monitoring.bind_addr`          |
| user-api       | `0.0.0.0:9202`           | `user_api.monitoring.bind_addr`             |
| block-indexer  | `0.0.0.0:9203`           | `block_indexer.monitoring.bind_addr`        |
| log-indexer    | `0.0.0.0:9204`           | `log_indexer.monitoring.bind_addr`          |

`user-api`'s `/metrics` endpoint lives on a **separate** port from the public
API (40001) so operators can firewall it from external traffic. The public API
is still instrumented (RED metrics, see below) — those samples are exposed
through the dedicated metrics port.

Each service section also accepts a boolean `monitoring.enabled` (defaults to
`true`); set it to `false` to skip starting the metrics endpoint without having
to remove the config block.

Override per environment via TOML or the `UB__` env-var hierarchy. Example:

```bash
UB__COORDINATOR__MONITORING__BIND_ADDR=127.0.0.1:19101 cli/run.sh coordinator
```

## Multi-operator port layout

`scripts/setup-operators.sh` can run several operators on one host. To avoid host
port collisions, each operator gets its own block of metrics ports offset by 10:
`base + (operator_index - 1) * 10`. Inside every operator's container the service
still binds the fixed `base.toml` port; only the **host** port is unique, so the
`docker-compose.yml` mapping is `host:container` = `<per-operator-port>:<base-port>`
(e.g. `9221:9201`).

Scrape each operator on its **host** port:

| Service       | op_1 | op_2 | op_3 | In-container (fixed) |
|---------------|------|------|------|----------------------|
| coordinator   | 9201 | 9211 | 9221 | 9201                 |
| user-api      | 9202 | 9212 | 9222 | 9202                 |
| block-indexer | 9203 | 9213 | 9223 | 9203                 |
| log-indexer   | 9204 | 9214 | 9224 | 9204                 |

```bash
curl -s http://localhost:9221/metrics   # op_3 coordinator (host port 9221)
```

## Global labels

Every sample carries a `service` label set by the binary at startup
(`coordinator`, `user-api`, `block-indexer`, `log-indexer`). This lets a single
Prometheus instance distinguish metrics that share a name across services
(`union_indexer_height`, `http_requests_total`, etc.).

## Checking metrics manually

Before wiring a full Prometheus instance, you can verify each service is
exporting samples with plain `curl`. Replace `localhost` with the operator host
if you're hitting a remote box, and keep in mind that `bind_addr` defaults to
`0.0.0.0` so the endpoint is reachable from anywhere unless you firewall it.

### 1. Smoke-test the endpoint is up

```bash
curl -sf http://localhost:9201/metrics > /dev/null && echo "coordinator OK"
curl -sf http://localhost:9202/metrics > /dev/null && echo "user-api OK"
curl -sf http://localhost:9203/metrics > /dev/null && echo "block-indexer OK"
curl -sf http://localhost:9204/metrics > /dev/null && echo "log-indexer OK"
```

`-f` makes curl exit non-zero on HTTP errors so you can chain it with `&&` in a
script. If a service prints nothing, check that `monitoring.enabled = true` in
its config and that the bind port isn't already taken.

### 2. Inspect a specific metric

`/metrics` returns plain text in the Prometheus exposition format. Filter with
`grep`:

```bash
# BitVMX liveness and last-message age (coordinator):
curl -s http://localhost:9201/metrics | grep -E '^union_bitvmx_(liveness|last_message)'

# Block indexer tip height:
curl -s http://localhost:9203/metrics | grep '^union_indexer_height'

# user-api RED metrics for the pegin endpoint:
curl -s http://localhost:9202/metrics | grep 'path="/user/pegin-address"'
```

Each non-comment line looks like:

```
union_bitvmx_liveness{service="coordinator"} 1
union_indexer_height{indexer="block",network="rsk",service="block-indexer"} 1234567
http_requests_total{method="POST",path="/user/pegin-address",service="user-api",status="200"} 42
```

The `service` global label is always present — verify it matches the binary
you're scraping so you don't confuse two endpoints.

### 3. Watch a metric live

For counters that increment as flows progress, `watch` makes the change
obvious:

```bash
watch -n 1 'curl -s http://localhost:9201/metrics | grep -E "^union_coordinator_events_processed_total"'
```

You should see the per-`kind` counters tick up as RSK blocks arrive and the
coordinator polls its monitors.

### 4. From inside the operator docker network

When services run via `docker/operator/docker-compose.yml`, each metrics port is
published to the host (see the per-operator layout above), so you can scrape it
straight from the host:

```bash
curl -s http://localhost:9201/metrics | head -20   # coordinator, op_1
```

To hit the in-container port directly instead (the fixed `base.toml` port), exec
into the container:

```bash
docker compose -f docker/operator/docker-compose.yml exec coordinator \
    curl -s http://localhost:9201/metrics | head -20
```

(Replace `curl` with `wget -qO-` if the image doesn't ship curl.)

### 5. Browser

Pasting `http://localhost:9201/metrics` into a browser also works — the response
is `Content-Type: text/plain`, so the browser will render it as-is. Useful for a
quick eyeball; not useful for filtering.

### 6. Validate Prometheus actually accepts the exposition

If you suspect a malformed metric (e.g. after adding new instrumentation),
[`promtool`](https://prometheus.io/docs/prometheus/latest/command-line/promtool/)
can lint a captured snapshot:

```bash
curl -s http://localhost:9201/metrics > /tmp/coordinator.prom
promtool check metrics < /tmp/coordinator.prom
```

A clean run prints nothing; any parse error points at the offending line.

## Metric catalogue

### RED metrics (user-api)

Recorded by the shared `track_http_metrics` middleware. The `path` label uses
axum's `MatchedPath` (the route template, e.g. `/user/pegin-address`) so
per-request URI parameters cannot inflate label cardinality.

| Metric                              | Type      | Labels                  |
|-------------------------------------|-----------|-------------------------|
| `http_requests_total`               | counter   | `method`, `path`, `status` |
| `http_request_duration_seconds`     | histogram | `method`, `path`        |

### Coordinator

| Metric                                            | Type    | Labels  | Notes                                                                 |
|---------------------------------------------------|---------|---------|-----------------------------------------------------------------------|
| `union_bitvmx_liveness`                           | gauge   |         | `1.0 = Healthy`, `0.0 = Unknown`, `-1.0 = NotResponding`              |
| `union_bitvmx_pings_sent_total`                   | counter |         | Pings the coordinator sent to BitVMX                                  |
| `union_bitvmx_ping_send_errors_total`             | counter |         | Broker send failures while attempting a BitVMX ping                   |
| `union_bitvmx_ping_timeouts_total`                | counter |         | Pings that did not get a reply within `bitvmx_not_responding_threshold_secs` |
| `union_bitvmx_messages_received_total`            | counter |         | Any inbound BitVMX message (pong or event)                            |
| `union_bitvmx_last_message_timestamp_seconds`     | gauge   |         | UNIX timestamp of the most recent inbound BitVMX message. Use `time() - <metric>` for age. |
| `union_coordinator_events_processed_total`        | counter | `kind`  | `kind` ∈ {`user_request`, `bitvmx_event`, `rsk_event`, `block`}       |
| `union_coordinator_processor_errors_total`        | counter | `kind`  | An `EventProcessor` (flow) returned an error while handling an event. Same `kind` set as above. Non-zero `rate` means a flow is failing. |
| `union_coordinator_broker_transport_errors_total` | counter | `operation` | Recoverable broker transport errors that were logged-and-continued (not fatal). `operation` ∈ {`getting User request`, `getting BitVMX event`, `getting RSK event`, `getting block`, `sending user reply`}. Sustained `rate` signals broker/connectivity degradation. |
| `union_coordinator_chain_height`                  | gauge   |         | RSK block number the coordinator last processed. Subtract from `union_indexer_height{indexer="block"}` to measure how far the coordinator lags the indexer. |
| `union_flows_active`                              | gauge   | `type`  | Refreshed every 10 RSK blocks (see `ACTIVE_FLOWS_LOG_EVERY_N_BLOCKS`). `type` ∈ {`pegin`, `pegout`, `advance-funds`, `committee-setup`} |

### Block indexer

| Metric                                       | Type    | Labels                         | Notes                                                              |
|----------------------------------------------|---------|--------------------------------|--------------------------------------------------------------------|
| `union_indexer_height`                       | gauge   | `indexer="block"`, `network`   | Last block number notified to consumers                            |
| `union_indexer_blocks_indexed_total`         | counter |                                | Each successful `notify_block`                                     |
| `union_indexer_reorgs_total`                 | counter | `indexer="block"`              | Increment when `backward_sync` replaces a canonical block          |
| `union_indexer_subscription_errors_total`    | counter | `indexer="block"`, `kind`      | `kind` ∈ {`closed`, `transient`, `lagged`, `unexpected`}           |
| `union_indexer_notify_errors_total`          | counter | `indexer="block"`              | Channel send failures while notifying downstream consumers         |

### Log indexer

| Metric                                          | Type    | Labels                         | Notes                                                            |
|-------------------------------------------------|---------|--------------------------------|------------------------------------------------------------------|
| `union_indexer_height`                          | gauge   | `indexer="log"`, `network`     | Block number of the most recently processed log                  |
| `union_log_indexer_logs_indexed_total`          | counter |                                | Each successful log persisted                                    |
| `union_log_indexer_unmanaged_contract_logs_total` | counter |                              | Logs received from an address outside `[[contracts]]`            |
| `union_indexer_subscription_errors_total`       | counter | `indexer="log"`, `kind`        | Same `kind` set as block indexer                                 |
| `union_indexer_notify_errors_total`             | counter | `indexer="log"`                | Channel send failures while notifying downstream consumers       |

### Process / runtime

The Prometheus exporter publishes the standard process gauges and counters
automatically (e.g. `process_cpu_seconds_total`, `process_resident_memory_bytes`,
`process_open_fds`). No code changes are needed to opt in.

## Cardinality policy

Do **not** introduce per-flow or per-transaction labels. `flow_id`, `tx_id`,
`block_hash`, addresses and other unbounded identifiers can blow up the time
series store. The closed enums used today (`FlowKind`, subscription error
variants) are safe. Per-flow tracing should go through `tracing` spans, not
metrics.

## Example `prometheus.yml`

```yaml
scrape_configs:
  - job_name: union-bridge-operator
    scrape_interval: 15s
    static_configs:
      - targets:
          - operator-host:9201  # coordinator
          - operator-host:9202  # user-api
          - operator-host:9203  # block-indexer
          - operator-host:9204  # log-indexer
```

### Multiple operators on one host

`setup-operators.sh` publishes each operator's metrics on the same host with
ports offset by 10 (see the multi-operator layout above). The global `service`
label distinguishes the four binaries but **not** the operators — every
operator's coordinator emits `service="coordinator"` — so attach an `operator`
label per target group:

```yaml
scrape_configs:
  - job_name: union-bridge
    scrape_interval: 15s
    static_configs:
      - targets: [operator-host:9201, operator-host:9202, operator-host:9203, operator-host:9204]
        labels: { operator: op_1 }
      - targets: [operator-host:9211, operator-host:9212, operator-host:9213, operator-host:9214]
        labels: { operator: op_2 }
      - targets: [operator-host:9221, operator-host:9222, operator-host:9223, operator-host:9224]
        labels: { operator: op_3 }
```

For more than a handful of operators, lift these groups into a file SD or
service-discovery source rather than hand-maintaining the list.

## Useful PromQL

- BitVMX outage (no message for 60 s):
  `time() - union_bitvmx_last_message_timestamp_seconds > 60`
- Tip lag of the block indexer vs. the BTC/RSK provider (compare against your
  own provider gauge or `max_over_time`):
  `max_over_time(union_indexer_height{indexer="block"}[5m]) - union_indexer_height{indexer="block"}`
- Active pegouts by step:
  `sum by (type) (union_flows_active{type="pegout"})`
- p95 user-API latency over the last 5 min:
  `histogram_quantile(0.95, sum by (le, path) (rate(http_request_duration_seconds_bucket{service="user-api"}[5m])))`
- A flow is failing (any processor error in the last 5 min):
  `sum by (kind) (rate(union_coordinator_processor_errors_total[5m])) > 0`
- Broker connectivity degrading (recoverable transport errors):
  `sum by (operation) (rate(union_coordinator_broker_transport_errors_total[5m])) > 0`
- Coordinator falling behind the block indexer (lag in blocks):
  `union_indexer_height{indexer="block"} - on() union_coordinator_chain_height`
- Block and log indexers out of sync with each other:
  `abs(union_indexer_height{indexer="block"} - on() union_indexer_height{indexer="log"})`
- A service stalled (height flat / no events while the chain advances):
  `rate(union_coordinator_events_processed_total{kind="block"}[5m]) == 0`
  `rate(union_indexer_blocks_indexed_total[5m]) == 0`
