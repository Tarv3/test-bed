# test-bed
Rust based test bed

## Config

```
[params]
files = ["file1", "file2", "file3"]
params = ["1", "2", "3"]

[commands]

for file in files {
    for param in params {
        spawn 0 ./program.exe --stdout=[file::files]_stdout.txt --stderr=[file::files]_stderr.txt
            --config [file::files].json
            --param [param::params];
        
        wait_for 0;
    }
}

wait_all;
```
