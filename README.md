# Preview
## Processes
![process](./data/preview/process.png)

## CPU
![cpu](./data/preview/cpu.png)

## Memory
![memory](./data/preview/memory.png)

## Disk
![disk](./data/preview/disk.png)

## Network
![network](./data/preview/network.png)

## Battery
![battery](./data/preview/battery.png)

# Why wrote this.
I like the design of https://github.com/nokyan/resources and https://missioncenter.io/, but I wanted to use them in a terminal, so I forked this tui tool based on resources.

# Credits
    - resources: https://github.com/nokyan/resources
    - btm: https://github.com/ClementTsang/bottom
    - missioncenter: https://missioncenter.io/

# Refresh interval

restop samples hardware and processes every 2000 ms by default. Override the
intervals independently with environment variables (values are clamped to
250–60000 ms):

```sh
RESTOP_UPDATE_MS=1000 RESTOP_PROCESS_UPDATE_MS=2000 restop
```
