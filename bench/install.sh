#!/bin/sh

set -e

for dir in ./others/*; do
  if [ -d "$dir" ]; then
    (
      if [ -f "$dir/install.sh" ]; then
        echo "Executing install script in $dir"
        cd "$dir"
        ./install.sh
      fi
    )
  fi
done

echo "All install scripts executed successfully."
