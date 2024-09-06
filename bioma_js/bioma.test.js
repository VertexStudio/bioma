// Test the Bioma interface

const BiomaInterface = require('./bioma');

const bioma = new BiomaInterface();

bioma.connect();


// const BiomaInterface = require('./BiomaInterface');

// async function main() {
//     const biomaInterface = new BiomaInterface();
//     try {
//         await biomaInterface.connect('http://127.0.0.1:8000/rpc', 'test', 'test');

//         const actorId = 'actor:123';
//         const message = { type: 'Ping' };

//         const reply = await biomaInterface.sendAndWaitForReply(actorId, message);
//         console.log('Received reply from Bioma actor:', reply);
//     } catch (error) {
//         console.error('Error interacting with Bioma:', error);
//     } finally {
//         await biomaInterface.close();
//     }
// }

// main().catch(console.error);