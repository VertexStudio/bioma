// Test the Bioma interface

const BiomaInterface = require('./bioma');

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
