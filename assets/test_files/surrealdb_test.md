# SurrealDB

SurrealDB is a modern, scalable, and versatile database designed for simplicity and performance. It supports a variety of data models, including graph, document, and relational, making it suitable for diverse application needs. With a focus on developer experience, SurrealDB simplifies complex database interactions while maintaining robust functionality.

## Why Choose SurrealDB?

SurrealDB offers several advantages:

- **Multi-Model Support:** Combines graph, document, and relational database capabilities in one.
- **Ease of Use:** Features an SQL-like query language that is easy to learn and use.
- **Scalability:** Designed for high performance, handling both small-scale and large-scale applications.
- **Realtime Capabilities:** Built-in support for realtime data subscriptions.
- **Flexible Deployment:** Supports on-premise, cloud, and edge environments.

## Key Features

- **Unified Query Language:** A familiar SQL-like syntax with added support for advanced graph queries.
- **Graph and Relational Models:** Native support for graph-based and relational queries, enabling complex relationships.
- **Realtime Updates:** Push-based data updates for seamless realtime applications.
- **Schema-less and Schema-full Support:** Offers flexibility for developers to define structured or unstructured data models.
- **Authentication and Security:** Built-in user authentication and role-based access control.

## Installation

SurrealDB can be easily installed using Docker or directly as a binary.

### Using Docker

```bash
docker pull surrealdb/surrealdb:latest
docker run --rm -p 8000:8000 surrealdb/surrealdb:latest start --log debug
```

### As a Binary

1. Download the latest release from the [official SurrealDB releases page](https://github.com/surrealdb/surrealdb/releases).
2. Extract and place the binary in your PATH.
3. Start the database server:

   ```bash
   surreal start --log debug
   ```

## Getting Started

1. **Start the Server:** Launch SurrealDB as shown in the installation steps.

2. **Connect to the Database:** Use the `surreal` CLI tool or a supported client library.

   ```bash
   surreal sql --conn http://localhost:8000 --user root --pass root
   ```

3. **Define and Insert Data:**

   ```sql
   CREATE person SET name = 'John Doe', age = 30, active = true;
   ```

4. **Query Data:**

   ```sql
   SELECT * FROM person WHERE active = true;
   ```

## SurrealDB Ecosystem

- **SurrealQL:** A versatile query language tailored for SurrealDB.
- **Official Client Libraries:** Available for languages such as Python, JavaScript, Go, and more.
- **Integration Support:** Compatible with modern frameworks and tools for seamless development.

## Community and Support

- **Official Documentation:** [https://surrealdb.com/docs](https://surrealdb.com/docs)
- **GitHub:** [https://github.com/surrealdb/surrealdb](https://github.com/surrealdb/surrealdb)
- **Community Forum:** Join discussions on the [SurrealDB Community Forum](https://community.surrealdb.com/).
- **Discord:** Connect with the community on the [SurrealDB Discord Server](https://discord.gg/surrealdb).

## Contributing

SurrealDB is open-source and welcomes contributions. To contribute:

1. Fork the [repository](https://github.com/surrealdb/surrealdb).
2. Review the [contribution guidelines](https://github.com/surrealdb/surrealdb/blob/main/CONTRIBUTING.md).
3. Submit a pull request with your improvements or bug fixes.

## License

SurrealDB is distributed under the [Business Source License](https://mariadb.com/bsl/).

---

Get started with SurrealDB today and experience a unified database solution that combines flexibility, performance, and ease of use.

