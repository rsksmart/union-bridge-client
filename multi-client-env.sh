#!/bin/bash

# Function to set environment variables for multi-client setup
set_multi_client_env() {
  local ID=$1
  local BASE_STORAGE_PATH=$2
  
  if [[ -z "${ID:-}" || ! "$ID" =~ ^([1-9]|10)$ ]]; then
    echo "Error: set_multi_client_env requires CLIENT_ID to be one of [1-10]" >&2
    return 1
  fi

  if [[ -z "${BASE_STORAGE_PATH:-}" ]]; then
    echo "Error: set_multi_client_env requires BASE_STORAGE_PATH argument" >&2
    return 1
  fi
  
  case "$ID" in
    1)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10001
      export UB__LOG_NOTIFIER__BROKER_PORT=11001
      export UB__BLOCK_BROKER__PORT=10001
      export UB__LOG_BROKER__PORT=11001
      export UB__USER_BROKER__PORT=30001
      export UB__BROKER_CLIENT_ID=101
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-1"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-1"
      export UB__SERVER__URL="0.0.0.0:9001"
      export UB__COORDINATOR_BROKER_CLIENT_ID=101
      export UB__BROKER_SERVER_PORT=30001
      export UB__HTTP_SERVER_PORT=40001
      export UB__BITVMX_BROKER__PORT=22222
      ;;
    2)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10002
      export UB__LOG_NOTIFIER__BROKER_PORT=11002
      export UB__BLOCK_BROKER__PORT=10002
      export UB__LOG_BROKER__PORT=11002
      export UB__USER_BROKER__PORT=30002
      export UB__BROKER_CLIENT_ID=102
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-2"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-2"
      export UB__SERVER__URL="0.0.0.0:9002"
      export UB__COORDINATOR_BROKER_CLIENT_ID=102
      export UB__BROKER_SERVER_PORT=30002
      export UB__HTTP_SERVER_PORT=40002
      export UB__BITVMX_BROKER__PORT=33333
      ;;
    3)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10003
      export UB__LOG_NOTIFIER__BROKER_PORT=11003
      export UB__BLOCK_BROKER__PORT=10003
      export UB__LOG_BROKER__PORT=11003
      export UB__USER_BROKER__PORT=30003
      export UB__BROKER_CLIENT_ID=103
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-3"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-3"
      export UB__SERVER__URL="0.0.0.0:9003"
      export UB__COORDINATOR_BROKER_CLIENT_ID=103
      export UB__BROKER_SERVER_PORT=30003
      export UB__HTTP_SERVER_PORT=40003
      export UB__BITVMX_BROKER__PORT=44444
      ;;
    4)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10004
      export UB__LOG_NOTIFIER__BROKER_PORT=11004
      export UB__BLOCK_BROKER__PORT=10004
      export UB__LOG_BROKER__PORT=11004
      export UB__USER_BROKER__PORT=30004
      export UB__BROKER_CLIENT_ID=104
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-4"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-4"
      export UB__SERVER__URL="0.0.0.0:9004"
      export UB__COORDINATOR_BROKER_CLIENT_ID=104
      export UB__BROKER_SERVER_PORT=30004
      export UB__HTTP_SERVER_PORT=40004
      export UB__BITVMX_BROKER__PORT=55554
      ;;
    5)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10005
      export UB__LOG_NOTIFIER__BROKER_PORT=11005
      export UB__BLOCK_BROKER__PORT=10005
      export UB__LOG_BROKER__PORT=11005
      export UB__USER_BROKER__PORT=30005
      export UB__BROKER_CLIENT_ID=105
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-5"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-5"
      export UB__SERVER__URL="0.0.0.0:9005"
      export UB__COORDINATOR_BROKER_CLIENT_ID=105
      export UB__BROKER_SERVER_PORT=30005
      export UB__HTTP_SERVER_PORT=40005
      export UB__BITVMX_BROKER__PORT=60005
      ;;
    6)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10006
      export UB__LOG_NOTIFIER__BROKER_PORT=11006
      export UB__BLOCK_BROKER__PORT=10006
      export UB__LOG_BROKER__PORT=11006
      export UB__USER_BROKER__PORT=30006
      export UB__BROKER_CLIENT_ID=106
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-6"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-6"
      export UB__SERVER__URL="0.0.0.0:9006"
      export UB__COORDINATOR_BROKER_CLIENT_ID=106
      export UB__BROKER_SERVER_PORT=30006
      export UB__HTTP_SERVER_PORT=40006
      export UB__BITVMX_BROKER__PORT=60006
      ;;
    7)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10007
      export UB__LOG_NOTIFIER__BROKER_PORT=11007
      export UB__BLOCK_BROKER__PORT=10007
      export UB__LOG_BROKER__PORT=11007
      export UB__USER_BROKER__PORT=30007
      export UB__BROKER_CLIENT_ID=107
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-7"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-7"
      export UB__SERVER__URL="0.0.0.0:9007"
      export UB__COORDINATOR_BROKER_CLIENT_ID=107
      export UB__BROKER_SERVER_PORT=30007
      export UB__HTTP_SERVER_PORT=40007
      export UB__BITVMX_BROKER__PORT=60007
      ;;
    8)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10008
      export UB__LOG_NOTIFIER__BROKER_PORT=11008
      export UB__BLOCK_BROKER__PORT=10008
      export UB__LOG_BROKER__PORT=11008
      export UB__USER_BROKER__PORT=30008
      export UB__BROKER_CLIENT_ID=108
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-8"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-8"
      export UB__SERVER__URL="0.0.0.0:9008"
      export UB__COORDINATOR_BROKER_CLIENT_ID=108
      export UB__BROKER_SERVER_PORT=30008
      export UB__HTTP_SERVER_PORT=40008
      export UB__BITVMX_BROKER__PORT=60008
      ;;
    9)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10009
      export UB__LOG_NOTIFIER__BROKER_PORT=11009
      export UB__BLOCK_BROKER__PORT=10009
      export UB__LOG_BROKER__PORT=11009
      export UB__USER_BROKER__PORT=30009
      export UB__BROKER_CLIENT_ID=109
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-9"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-9"
      export UB__SERVER__URL="0.0.0.0:9009"
      export UB__COORDINATOR_BROKER_CLIENT_ID=109
      export UB__BROKER_SERVER_PORT=30009
      export UB__HTTP_SERVER_PORT=40009
      export UB__BITVMX_BROKER__PORT=60009
      ;;
    10)
      export UB__BLOCK_NOTIFIER__BROKER_PORT=10010
      export UB__LOG_NOTIFIER__BROKER_PORT=11010
      export UB__BLOCK_BROKER__PORT=10010
      export UB__LOG_BROKER__PORT=11010
      export UB__USER_BROKER__PORT=30010
      export UB__BROKER_CLIENT_ID=110
      export UB__INDEXER__STORAGE__PATH="${BASE_STORAGE_PATH}/.union_bridge/database/multi-client-10"
      export UB__KEY_STORE__PATH="${BASE_STORAGE_PATH}/.union_bridge/keystore/multi-client-10"
      export UB__SERVER__URL="0.0.0.0:9010"
      export UB__COORDINATOR_BROKER_CLIENT_ID=110
      export UB__BROKER_SERVER_PORT=30010
      export UB__HTTP_SERVER_PORT=40010
      export UB__BITVMX_BROKER__PORT=60010
      ;;
  esac
  
  export CLIENT_ID=$ID
}
