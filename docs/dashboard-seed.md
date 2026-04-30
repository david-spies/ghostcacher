## Once the stack is up and you're connected to Grafana with prometheous.

Dashboard will be named 'GhostCacher — Distributed KV Cache'

From the same terminal - 

Seed the dashboard with your first real cache hit:

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


## Seeding errors - 

The terminal errors:
Issue 1 — bashexport: command not found

You accidentally typed bash directly before the export commands, so the shell tried to run a program called bashexport. Simply run the exports as separate commands:

export ANTHROPIC_API_KEY="sk-ant-YOUR_KEY_HERE"
export ANTHROPIC_BASE_URL="http://localhost:8080"

## JSONDecodeError

Issue 2 — JSONDecodeError: Expecting value: line 1 column 1
The curl response was empty — meaning the sidecar proxied the request to Anthropic but got back nothing, which happens when the API key is literally sk-ant-YOUR_KEY_HERE instead of your real key. The sidecar forwarded it, Anthropic rejected it with an auth error, and the sidecar returned an empty or non-JSON body.

## See ./docs/api-testing.md


