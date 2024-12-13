# Bioma LLM  

## Running stress tests  

This test is intented to test the performance of the endpoints in the rag server under stress conditions. With one command you can decide which endpoints you want to test.  

```bash
cargo run -p bioma_llm --example attack -- --endpoints <endpoints_list> --order
```  

```endpoints_list``` is a string with the endpoints you want to test separated by commas. The ```--order``` flag is optional and if you use it the endpoints will be tested in the order you passed them. For example:  

```bash
cargo run -p bioma_llm --example attack -- --endpoints health,hello,upload --order
```

will test, for every iteration, the health, hello and upload endpoints in this order.

This is a more complete example:

```bash
cargo run -p bioma_llm --example attack -- --endpoints health,hello,upload,index,ask --users 20 --time 70 --order --variations 16
```

To see all the arguments you can use, and the corresponding descriptions, please see the table below:

| Argument           | Required | Description                                                              | Example                                   | Default               |
|--------------------|----------|--------------------------------------------------------------------------|-------------------------------------------|-----------------------|
| `endpoints`        | `False`    | Endpoints to run with optional weights (endpoint:weight,endpoint:weight) | `--endpoints health:1,index:3,upload,ask` | all                   |
| `server`           | `False`    | RAG Server URL                                                           | `--server http://localhost:5766`          | http://localhost:5766 |
| `users`            | `False`    | Number of users in goose                                                 | `--users 15`                              | 10                    |
| `time`             | `False`    | Run time in seconds                                                      | `--time 5`                                | 60                    |
| `log`              | `False`    | Request log file path                                                    | `--log /tmp/requests.log`                 | .output/requests.log  |
| `report`           | `False`    | Report file path                                                         | `--report /tmp/report.html`               | .output/report.html   |
| `metrics-interval` | `False`    | Metrics reporting interval in seconds                                    | `--metrics-interval 4`                    | 15                    |
| `--order`          | `False`    | Run scenarios in sequential order                                        | `--order`                                 | -                     |
| `variations`       | `False`    | Number of variations for the test files                                  | `--variations 10`                         | 1                     |

Available endpoints:

- health
- hello
- index
- chat
- upload
- deletesource
- embed
- ask
- retrieve
- rerank
- all
