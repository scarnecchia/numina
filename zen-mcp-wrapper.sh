#!/usr/bin/env bash

nix-shell -p uv.out --run 'uvx --from git+https://github.com/BeehiveInnovations/zen-mcp-server.git zen-mcp-server'
