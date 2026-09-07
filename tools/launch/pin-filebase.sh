#!/usr/bin/env bash
# Pin a CAR file's DAG on Filebase by importing the CAR through its S3 API, then
# confirm that the service reports the same root CID the CAR carries.
#
#   pin-filebase.sh <file.car> <bucket> [object-key]
#
# Credentials come from the owner's own AWS CLI configuration (AWS_PROFILE or
# AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY for the Filebase key pair); this script
# never reads or prints them. Any pinning service that imports CARs works the
# same way; see README.md for the Storacha alternative.
set -euo pipefail
CAR=${1:?usage: pin-filebase.sh <file.car> <bucket> [object-key]}
BUCKET=${2:?usage: pin-filebase.sh <file.car> <bucket> [object-key]}
KEY=${3:-$(basename "$CAR")}
HERE=$(cd "$(dirname "$0")" && pwd)
ENDPOINT=${FILEBASE_ENDPOINT:-https://s3.filebase.com}
ROOT=$("$HERE/verify-car.sh" "$CAR" | tail -1 | awk '/^ok /{print $2}')
[ -n "$ROOT" ] || { echo "the CAR does not verify; refusing to upload" >&2; exit 1; }
aws --endpoint-url "$ENDPOINT" s3 cp "$CAR" "s3://$BUCKET/$KEY" --metadata import=car
# The AWS CLI title-cases user metadata keys ("Cid"); read the whole map and pick the key case-insensitively.
REPORTED=$(aws --endpoint-url "$ENDPOINT" s3api head-object --bucket "$BUCKET" --key "$KEY" --query 'Metadata' --output json | python3 -c 'import json,sys; m=json.load(sys.stdin) or {}; print(next((v for k,v in m.items() if k.lower()=="cid"), ""))')
STATUS=$(aws --endpoint-url "$ENDPOINT" s3api head-object --bucket "$BUCKET" --key "$KEY" --query 'Metadata' --output json | python3 -c 'import json,sys; m=json.load(sys.stdin) or {}; print(next((v for k,v in m.items() if k.lower()=="pinning-status"), ""))')
if [ "$REPORTED" = "$ROOT" ]; then
  echo "pinned $ROOT as s3://$BUCKET/$KEY (pinning status: ${STATUS:-unknown})"
else
  echo "MISMATCH: CAR root $ROOT, service reports $REPORTED" >&2; exit 1
fi
