"""Report environment names and value lengths without exposing values."""
import json
import os

if __name__ == '__main__':
    print(json.dumps({key: len(value) for key, value in sorted(os.environ.items())}, sort_keys=True))
