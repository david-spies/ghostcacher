## API - Testing

All development files posted to GitHub were used and verified in development testing. The stack is fully operational. Prometheus shows both targets active, the control plane is responding, and Grafana is displaying data.

## JSONDecodeError

Issue 2 — JSONDecodeError: Expecting value: line 1 column 1
The curl response was empty — meaning the sidecar proxied the request to Anthropic but got back nothing, which happens when the API key is literally sk-ant-YOUR_KEY_HERE instead of your real key. The sidecar forwarded it, Anthropic rejected it with an auth error, and the sidecar returned an empty or non-JSON body.

Run this with your real key:
# Set your actual key
export ANTHROPIC_API_KEY="sk-ant-api03-..."   # your real key here

# Test with verbose output to see exactly what comes back

```
curl -s http://localhost:8080/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: $ANTHROPIC_API_KEY" \
  -H "anthropic-version: 2023-06-01" \
  -d '{
    "model": "claude-haiku-4-5-20251001",
    "max_tokens": 32,
    "system": "You are a terse assistant.",
    "messages": [{"role": "user", "content": "Reply with: cache test OK"}]
  }'

```

Once that returns a valid JSON response you can run the 4-request loop to seed the hit ratio. Everything else — Prometheus targets, Grafana dashboard, control plane stats, Redis — is confirmed working. You just need a valid API key in the environment to get actual LLM responses flowing through and populating the metrics.

## Seed the dashboard with your first real cache hit (4-request loop):

```

export ANTHROPIC_API_KEY="sk-ant-YOUR_KEY_HERE"
export ANTHROPIC_BASE_URL="http://localhost:8080"

# Send the same system prompt 4 times — first is a MISS, rest are HITs
for i in 1 2 3 4; do
  curl -s http://localhost:8080/v1/messages \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANTHROPIC_API_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -d "{
      \"model\": \"claude-haiku-4-5-20251001\",
      \"max_tokens\": 32,
      \"system\": \"You are a terse legal AI assistant specializing in contract analysis.\",
      \"messages\": [{\"role\": \"user\", \"content\": \"Request $i: reply with OK\"}]
    }" | python3 -c "import sys,json; r=json.load(sys.stdin); print('Request $i:', r.get('content',[{}])[0].get('text','error'))"
done

```
