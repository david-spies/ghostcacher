Force a full rebuild and bring the stack up
The Docker cache has the old binary baked in. Force a fresh build:

bash

./scripts/dev.sh down

# Force rebuild — no cache

docker compose build --no-cache sidecar control-plane

./scripts/dev.sh up
