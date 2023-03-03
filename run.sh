#!/bin/bash

set -e

# Pseudocode of what this bash script does
# 1. Build the executable
# 2. Run 200 instances of the executable in a loop
# 3. Once the first executable fails (returns non zero error code),
#    kill all the rest of the instances
# 4. Save the log from the failed exetutable in a particular location and open
#    it with vim

# Directories used in this test
rm -rf /tmp/.tmp*
rm -rf /tmp/ouisync-test-*

function date_tag {
    date +"%Y-%m-%d--%H-%M-%S"
}

echo "$(date_tag) Compiling the test"
cargo build

# This next one will not compile again, we need it to get the executable name.
exe=./target/debug/database-bug

dir=`mktemp --tmpdir -d ouisync-test-XXXXXX`
n=200

echo "$(date_tag) Working in directory $dir"
echo "$(date_tag) Starting $n tests"

for i in $(seq $n); do
    while true; do
        $exe 2>&1
        r=$?
        if [ "$r" -ne "0" ]; then break; fi
    done > $dir/test-$i.log 2>&1 &

    pids[${i}]=$!
done

aborted_process="";

echo "$(date_tag) Awaiting first process to fail"

while [ -z "$aborted_process" ]; do
    for i in $(seq $n); do
        pid=${pids[${i}]}
        if ! ps $pid > /dev/null; then
            echo "Process $i (pid:$pid) aborted"
            aborted_process=$i
            break;
        fi
    done
    sleep 0.5
done

echo "$(date_tag) Killing rest of the jobs:"

for i in $(seq $n); do
    pid=${pids[${i}]}
    if [ $i -ne $aborted_process ]; then
        echo "  killing job:$i with pid:$pid"
        pkill -P $pid 2>/dev/null 1>&2 & # || true 
        rm $dir/test-$i.log
    fi
done

new_log_name=/tmp/ouisync-log-$(date +"%Y-%m-%d--%H-%M-%S").txt
mv $dir/test-$aborted_process.log $new_log_name
echo "$(date_tag) Log saved to $new_log_name"

vim $new_log_name
