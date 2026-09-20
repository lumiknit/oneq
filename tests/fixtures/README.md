# 1q test fixtures

## jq Codes

- `scripts`: Contains `jq` script file for testing. All scripts are written in `.jq` format, and run successfully with `jq`.
- `scripts-invalid`: Contains invalid `jq` script file. If you run them with `jq`, it will return error and exit soon.

## Data

- `jsons`: Contains `.jsons` files for testing.
  - As `jq` parse multiple JSON objects in a single stream, `.jsons` file also has multiple JSON objects in a single file.
  - Each value may be separated by a newline, spaces or word boundary.
- `texts`: Contains text files for testing. This is for test '-R' option, which read raw text instead of JSON objects.
