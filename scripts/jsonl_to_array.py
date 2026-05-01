#!/usr/bin/env python3
import json
import sys

# Varsayılan dosya yolu
input_path = sys.argv[1] if len(sys.argv) > 1 else "test_results.json"
output_path = sys.argv[2] if len(sys.argv) > 2 else "test_results_array.json"

results = []
with open(input_path, "r") as f:
    for line in f:
        line = line.strip()
        if line and line.startswith("{"):
            try:
                results.append(json.loads(line))
            except Exception:
                pass  # Derleme çıktısı veya uymayan satırları atla

with open(output_path, "w") as f:
    json.dump(results, f, indent=2)

print(f"[OK] {output_path} dosyasına {len(results)} JSON obje yazıldı.")
