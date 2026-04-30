## Troubleshooting Launch

### Permissions denied error

The Permission denied error in your terminal isn't a bug in the GhostCacher code; it’s a standard Linux filesystem security feature. When you clone a repository from GitHub, the scripts inside aren't always marked as "executable" by your local operating system.

Since dev.sh is the entry point for your entire Docker stack, the shell (bash) is refusing to run it because the "execute" bit isn't set for your user.
The Immediate Fix

Run the following command in your terminal from the ~/ghostcacher directory:

```
chmod +x scripts/dev.sh
```

What this does: chmod (change mode) with the +x flag tells Linux to treat this file as an executable program rather than a plain text file.
Why this happens (The Engineer's Perspective)

In Unix-like systems (Linux, macOS), every file has a set of permissions for Read (r), Write (w), and Execute (x).

When you ran ./scripts/dev.sh up, the system checked the metadata for that file. If you run ls -l scripts/dev.sh, you likely saw something like this:
-rw-r--r-- (The missing x means no one can execute it).

After running the chmod command, it will look like this:
-rwxr-xr-x (The x confirms it is now executable).
Best Practices for Your Environment

Since you are setting up a stack involving Cloud Security and SaaS infrastructure, here are two professional tips to keep your environment clean:
1. Check other scripts

If dev.sh lacked permissions, the smoke_test.py or other utility scripts might too. You can apply the executable bit to the entire scripts folder safely:

```
chmod +x scripts/*.sh
```

2. Verify Docker Permissions

Once you fix the script permission, you might hit a second common hurdle: Docker socket permissions. If dev.sh up fails with an error about "connecting to the Docker daemon," you need to run your command with sudo or add your user to the docker group:

```
sudo ./scripts/dev.sh up
```

Next Step

After running chmod +x scripts/dev.sh, try the launch command again:

```
./scripts/dev.sh up
```

This should successfully trigger the Docker Compose build and bring up your Redis, Sidecar, and Control Plane services.


### Ghost in the Machine

1. The "Ghost" in the Machine

When you run a command through GhostCacher, the traffic flow looks like this:

    Your Curl Client sends the request to localhost:8080 (The Sidecar).

    The Sidecar looks for your API Key to pass it along to Anthropic.

    Because $ANTHROPIC_API_KEY is empty in your terminal, the Sidecar sends an empty key, and Anthropic rejects it.

2. The Fix

You need to actually set your API key in the terminal session before running the curl.

Step 1: Set your key (Replace the placeholder with your actual key):

```
export ANTHROPIC_API_KEY=sk-ant-api03-xxxxxxxxxxxxxxxxx
```

Step 2: Run the corrected Curl
Notice that since you are using the Sidecar as a proxy, you should point your curl at localhost:8080 instead of the direct Anthropic URL. That is how GhostCacher actually "catches" the request:

```
curl http://localhost:8080/v1/messages \
     -H "x-api-key: $ANTHROPIC_API_KEY" \
     -H "anthropic-version: 2023-06-01" \
     -H "content-type: application/json" \
     -d '{
       "model": "claude-3-5-sonnet-20240620",
       "max_tokens": 1024,
       "messages": [{"role": "user", "content": "Hello, GhostCacher!"}]
     }'
```

3. How to tell if it's working

Run that command twice.

    1st time: You'll see a slight delay. This is a "Cache Miss." The Sidecar is talking to Anthropic and saving the response to Redis.

    2nd time: The response should be near-instant. That is a "Cache Hit."

If you want to be 100% sure, check your Docker logs in another terminal window while you run the curl:

```
docker logs -f gc-sidecar
```

You should see logs appearing that say something like [CACHE] Miss - Fetching from upstream or [CACHE] Hit - Serving from Redis.

### Port Conflict

Port Conflict.

The Problem

The error address already in use for port 6379 means you already have a version of Redis running directly on your host machine. Because the docker-compose.yml is trying to grab that same port for the gc-redis container, they are bumping heads.
The Fix

You have two main options:

Option A: Stop the local Redis (Recommended)
If you aren't using the local Redis for anything else right now, just turn it off so Docker can have the port:

```
sudo systemctl stop redis-server
```

Then try starting your stack again:

```
./scripts/dev.sh up
```

Option B: Change the Port Mapping
If you need that local Redis running, you'll need to edit your docker-compose.yml file. Look for the redis service and change the port mapping:

```
# Change this:
ports:
  - "6379:6379"

# To this (connect via 6380 on your host instead):
ports:
  - "6380:6379"
```

How to verify it worked

Once you run ./scripts/dev.sh up again and it finishes without errors, check that everything is healthy:

```
docker ps
```

You should see gc-control-plane, gc-sidecar, gc-redis, gc-prometheus, and gc-grafana all listed as "Up".

- Once Port 6379 is clear, and your GhostCacher environment is officially running, your terminal should display the following:

Here is a quick breakdown of your new playground:

🛠️ The GhostCacher Stack

Service	        URL	                    Purpose
Sidecar Proxy	http://localhost:8080	The entry point for your LLM requests.
Control Plane	http://localhost:7070	Managing cache logic and hit/miss stats.
Grafana	        http://localhost:3000	Visualizing cache performance (User: admin, Pass: ghostcacher).
RedisInsight	http://localhost:8001	GUI to peek inside the Redis cache and see what's stored.

### - docker-compose.yml: `version` is obsolete

The error permission denied while trying to connect to the Docker daemon socket means your current shell session doesn't have the authority to talk to Docker.
The Quick Fix

You can bypass this immediately by using sudo:

```
sudo ./scripts/dev.sh up
```

The Permanent Fix (Recommended)

If you don't want to type sudo every single time you touch GhostCacher, you need to add your user to the docker group. This is standard procedure for a dev machine.

1. Add your user to the docker group:

```
sudo usermod -aG docker $USER
```

2. Apply the changes to your current session:

```
newgrp docker
```

After running those two commands, ./scripts/dev.sh up should work perfectly without needing sudo.
Why did this happen?

Docker communicates through a Unix socket (/var/run/docker.sock). By default, only the root user has read/write access to that socket. Adding yourself to the docker group is like getting a VIP badge that lets you bypass the "Root Only" security gate.

Give that newgrp command a shot and try starting the stack again—you should be back in the pilot's seat!



