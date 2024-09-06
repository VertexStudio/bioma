# Minimal interface for Bioma

This crate provides a minimal interface for Bioma, allowing you to interact with Bioma from JavaScript.


## Example
```js
const bioma = new BiomaInterface();

async function main() {

    await bioma.connect();

    const dummyId = bioma.createActorId('/dummy', 'dummy::Dummy');
    const dummyActor = await bioma.createActor(dummyId);

    const echoActorId = bioma.createActorId('/echo', 'echo::Echo');
    const echoMessage = { text: 'Hello, world!' };

    let messageId = await bioma.sendMessage(dummyId, echoActorId, 'echo::EchoText', echoMessage);

    let reply = await bioma.waitForReply(messageId);

    console.log('Reply:', reply);

    await bioma.close();
}

main();
```

Expected output:

```bash
Connected to Bioma SurrealDB
Message sent to: actor:⟨/echo⟩
Reply: {
  err: undefined,
  id: RecordId { tb: 'reply', id: '01J72WZDCNVNE1D5P3J3KQ5CP4' },
  msg: { echoes_left: 0, text: 'Hello, world!' },
  name: 'echo::EchoText',
  rx: RecordId { tb: 'actor', id: '/dummy' },
  tx: RecordId { tb: 'actor', id: '/echo' }
}
Disconnected from Bioma SurrealDB
```


## Install

```bash
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.1/install.sh | bash
nvm install node

cd bioma_js
node install
```

## Test

```bash
cd bioma_js
node bioma.test.js
```


