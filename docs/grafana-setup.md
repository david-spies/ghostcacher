## Grafana Setup

Once your 'Stack is up.'

[+] Running 5/3
 ✔ Container gc-redis          Healthy                                                         0.0s 
 ✔ Container gc-prometheus     Running                                                         0.0s 
 ✔ Container gc-grafana        Running                                                         0.0s 
 ✔ Container gc-control-plane  Started                                                         0.0s 
 ✔ Container gc-sidecar        Started                                                         0.0s 
[OK] Stack is up.

  Sidecar proxy:     http://localhost:8080
  Control plane:     http://localhost:7070/gc/stats
  Prometheus:        http://localhost:9091
  Grafana:           http://localhost:3000  (admin/ghostcacher)
  RedisInsight:      http://localhost:8001
  Set your LLM client base URL:
  export ANTHROPIC_BASE_URL=http://localhost:8080

Open a broswer tab and navigate to the following address:

```
http://localhost:3000
```

user: admin
password: ghostcacher

The Grafana dashboard should now be displayed.

- Add a data source

Under 'Connections'

Select 'Add new connection'

Select 'Prometheus'

Click 'Add new data source'

Prometheus server URL: http://prometheus:9090

Click 'Save & Test'

Upper right corner: Click 'Build Dashboard'

Click 'Import a dashboard'

Upload dashboard JSON file OR Drag-n-Drop 'grafana-dashboard.json'

Name, Folder, and Unique identifier (UID) will auto-populate.

Click on 'Import'

## Empty Dashboard

The empty dashboard is expected — Prometheus has nothing to scrape yet because no LLM requests have flowed through the sidecar. Here's the exact sequence to go from zero data to a live dashboard.

- Step 1 — Verify Prometheus is actually reaching the sidecar
Open http://localhost:9091/targets in your browser. You should see two targets:

> ghostcacher-sidecar at sidecar:9090
> ghostcacher-control-plane at control-plane:7070

Both should show State: UP in green. If either shows DOWN, that's your root cause — jump to the troubleshooting block below.

- Step 2 — Seed the first metrics by sending a real request
Prometheus panels stay blank until at least one metric exists. The fastest way is to fire a test request through the sidecar proxy:

# Confirm the sidecar is alive first
curl http://localhost:8080/healthz
# {"status":"ok","service":"ghostcacher-sidecar"}

# Fire a real proxied request (replace with your actual API key)
export ANTHROPIC_API_KEY="sk-ant-..."
export ANTHROPIC_BASE_URL="http://localhost:8080"

python3 - <<'EOF'
import anthropic, os

client = anthropic.Anthropic()
r = client.messages.create(
    model="claude-haiku-4-5-20251001",
    max_tokens=64,
    system="You are a terse assistant.",
    messages=[{"role":"user","content":"Reply with: cache test one"}],
)
print(r.content[0].text)
EOF

Send it three or four times with the same system prompt — the second hit should produce a cache HIT, which is what lights up the gc_cache_hit_ratio and gc_saved_ttft_ms_total panels.

- Step 3 — Confirm metrics are actually being emitted

# Raw Prometheus scrape from the sidecar metrics port
curl -s http://localhost:9090/metrics | grep gc_

# Or query via the Prometheus HTTP API
curl -s "http://localhost:9091/api/v1/query?query=gc_cache_hits_total" | python3 -m json.tool

You should see output like:
```
gc_cache_hits_total 3
gc_cache_misses_total 1
gc_cache_hit_ratio 0.75
gc_tokens_cached_total 1820
```

If those values exist in Prometheus, Grafana will graph them.
Step 4 — Confirm the datasource UID matches the dashboard
This is the most common silent failure. Open monitoring/grafana-dashboard.json and look near the top for:

"datasource": {
  "type": "prometheus",
  "uid": "PBFA97CFB590B2093"
}

Then in Grafana go to Connections → Data sources → Prometheus and check the UID shown in the URL bar (it looks like http://localhost:3000/datasources/edit/abc123). If the UID in the JSON doesn't match what Grafana assigned, panels silently show "No data."
Fix it two ways — easiest is to update the dashboard JSON before importing:

# Get the UID Grafana actually assigned to your Prometheus source
GRAFANA_DS_UID=$(curl -s http://admin:ghostcacher@localhost:3000/api/datasources \
  | python3 -c "import sys,json; ds=json.load(sys.stdin); print(next(d['uid'] for d in ds if d['type']=='prometheus'))")

echo "Prometheus datasource UID: $GRAFANA_DS_UID"

# Patch the dashboard JSON with the real UID
sed -i "s/PBFA97CFB590B2093/$GRAFANA_DS_UID/g" monitoring/grafana-dashboard.json

Then re-import the patched JSON in Grafana (Dashboards → Import → Upload JSON).
Step 5 — Set the time range
Grafana defaults to "Last 6 hours." If you just started the stack, there's nothing in that window. In the top-right corner of the dashboard, change the range to Last 15 minutes and click the refresh icon. Data should appear immediately after Step 2.
No other data sources needed. The dashboard is Prometheus-only. Redis, the sidecar, and the control plane all funnel into Prometheus — Grafana reads from there alone. RedisInsight (http://localhost:8001) is a separate UI for inspecting Redis keys directly; it's useful for debugging cache entries but is not wired into Grafana.


Quick troubleshooting reference

```
Symptom                                          Likely cause                            Fix
Prometheus target shows DOWN                     Sidecar container not exposing :9090    docker compose logs gc-sidecar — check for startup errors
Metrics endpoint returns empty                   No requests sent yet                    Run Step 2
Panels show "No data" despite metrics existing   Datasource UID mismatch                 Run Step 4
Panels show "No data" despite correct UID        Time range too wide                     Set to Last 15 minutes
gc_cache_hit_ratio always 0                      Only one request sent                   Send 3–4 identical-prefix requests
```

Once you see gc_cache_hit_ratio climbing past 0 in the top-left stat panel, the full dashboard is live. From there the TTFT savings, tokens cached, and hit/miss rate chart will all populate as traffic flows through.





