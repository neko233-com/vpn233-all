#!/bin/bash

BINARY="./vpn233-client"
CONFIG="config.json"
LOG="vpn233-client.log"
PID_FILE="vpn233-client.pid"

usage() {
    echo "Usage: $0 {start|stop|status|restart}"
    exit 1
}

start() {
    if [ -f $PID_FILE ]; then
        PID=$(cat $PID_FILE)
        if ps -p $PID > /dev/null; then
            echo "[INFO] $BINARY is already running (PID: $PID)."
            return 0
        fi
        rm $PID_FILE
    fi

    echo "[INFO] Starting $BINARY..."
    # Note: TUN mode requires sudo/root privileges.
    nohup $BINARY -c $CONFIG > $LOG 2>&1 &
    PID=$!
    echo $PID > $PID_FILE
    sleep 2
    if ps -p $PID > /dev/null; then
        echo "[SUCCESS] $BINARY started successfully (PID: $PID)."
    else
        echo "[ERROR] Failed to start $BINARY. Check $LOG for details."
        rm $PID_FILE
    fi
}

stop() {
    if [ ! -f $PID_FILE ]; then
        echo "[INFO] $BINARY is not running (no PID file)."
        return 0
    fi

    PID=$(cat $PID_FILE)
    echo "[INFO] Stopping $BINARY (PID: $PID)..."
    kill $PID
    sleep 2
    if ps -p $PID > /dev/null; then
        echo "[INFO] $BINARY did not stop, killing forcefully..."
        kill -9 $PID
    fi
    rm $PID_FILE
    echo "[SUCCESS] $BINARY stopped."
}

status() {
    if [ -f $PID_FILE ]; then
        PID=$(cat $PID_FILE)
        if ps -p $PID > /dev/null; then
            echo "[STATUS] $BINARY is RUNNING (PID: $PID)"
            return 0
        fi
    fi
    echo "[STATUS] $BINARY is STOPPED"
}

case "$1" in
    start)
        start
        ;;
    stop)
        stop
        ;;
    status)
        status
        ;;
    restart)
        stop
        start
        ;;
    *)
        usage
        ;;
esac
