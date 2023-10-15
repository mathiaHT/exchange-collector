# Exchange collector

![build](https://github.com/mathiaHT/exchange-collector/actions/workflows/rust/badge.svg)
![docker](https://github.com/mathiaHT/exchange-collector/actions/workflows/docker/badge.svg)

A crypto exchange collector written in rust to

- listen exchanges websocket
- compute technical analysis indicator in real time
- generate market event based on indicators
- write trading events to [delta table][deltalake]

## How to use the collector locally

1. Launch the services `docker-compose up -d`
2. Go to [grafana][dashboard], user is "admin" and password "admin"
3. List available data `awslocal s3 ls s3://datalake/raw/`
3. List market events `awslocal dynamodb scan --table-name event-table`

[deltalake]: https://docs.rs/crate/deltalake/latest
[dashboard]: http://localhost:3000/d/matches/matches?orgId=1&refresh=5s
