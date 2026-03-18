I removed a blob from the remote directory so pull whatever changes you need or stash any changes that aren't committed and then run a force fetch to remove the blob if needed or just delete it from local memory. 
with git fetch --all && git reset --hard origin/main

Just so we don't push the same blob

---


