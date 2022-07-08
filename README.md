# test-bed
Rust based test bed

## Config

```json5

{
  // Each combination of indices will be executed once, i.e., 3 * 2 * 100 iterations will occur
  "indices": [3, 2, 100],
  
  // These params can be used in conjunction with the indices to execute programs with different parameters
  "params": {
    "config": ["file.json", "file2.json"],
    "simulations": ["1000", "100", "10"]
  },
  
  // The sequence of commands is exectuted for each iterations
  "commands": [
    // runs "./test.exe -a arg1 --config [2::config] --sims [0::simulations]", writing stdout into "out.txt" and stderr into "file.txt"
    // Assigns id '1' to this process
    "spawn 1 --stdout=append(out.txt) --stderr=file.txt ./test.exe -a arg1 --config [2::config] --sims [0::simulations]",
    
    // Waits for process with id '1' to finish. Will timeout after '100' ms and try '5' times.
    "wait 1 100 5",
    
    // Forcefully kill the process with id '1' if it is still running
    "kill 1",
    
    // Sleep the test bed for '500' ms
    "sleep 500"
  ]
}
```
