#!/bin/sh

set -e

kill_process() {
  pid_to_kill=$1
  if [ -n "$pid_to_kill" ]; then
    if kill -0 "$pid_to_kill" 2>/dev/null; then
      echo "Stopping process with PID: $pid_to_kill"
      kill "$pid_to_kill"
      wait "$pid_to_kill" 2>/dev/null
    fi
  fi
}

cleanup() {
  echo "Cleaning up background processes..."
  kill_process "$CURRENT_TEST_PID"
  kill_process "$SUBGRAPHS_PID"
}

trap cleanup EXIT INT

cargo build -r -p gateway -p subgraphs

mkdir -p summaries

../target/release/subgraphs &
SUBGRAPHS_PID=$!

echo "Starting Hive RS"
../target/release/gateway ./supergraph.graphql &
GATEWAY_PID=$!
sleep 5
k6 run k6.js > ./summaries/hive-rs.log
kill_process "$GATEWAY_PID"
echo "Finished Hive RS"

for dir in ./others/*; do
  if [ -d "$dir" ]; then
    (
      if [ -f "$dir/run.sh" ]; then
        GATEWAY_NAME=$(basename "$dir")
        echo "Running $GATEWAY_NAME"
        cd "$dir"
        ./run.sh &
        GATEWAY_PID=$!
        sleep 5
        k6 run k6.js > "../summaries/$GATEWAY_NAME.log"
        kill_process "$GATEWAY_PID"
        echo "Finished $GATEWAY_NAME"
      fi
    )
  fi
done

echo "---"
echo "Finished all benchmarks"
echo "---"


# print content of summaries
for file in summaries/*; do
  echo "File: $file"
  cat "$file"
done
