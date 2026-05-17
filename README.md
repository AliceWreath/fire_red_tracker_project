# Fire Red Project Files
This project requires that you have a copy of FireRed r1 for it to work correctly.

# Fire Red Monitor
This is the primary program for viewing the party and encounter data.
It can run in standalone, client, or server mode.
Standalone: ./executable /path/to/rom.gba
Client: ./executable /path/to/rom.gba --client addr:port
Server: ./executable /path/to/rom.gba --server port
  
# Standalone
  This will create 2 windows one for party data and one for encounter data.
# Client
  This will create 2 windows one for party data and one for encounter data, and get all its
  data from an executable running in server mode
# Server
  This will generate the data but not display it. To view the data, connect with an executable
  in Client Mode or use the Aggregator program

# Fire Red Aggregator
This program connects to up to 4 Fire Red Monitor executables in server mode.
It takes in their data and formats it all into a single window.
syntax: ./executable /path/to/rom --ip:port --ip:port ...etc
