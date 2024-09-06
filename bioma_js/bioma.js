const { Surreal } = require('surrealdb.js');
const crypto = require('crypto');

class BiomaInterface {
    constructor() {
        this.db = new Surreal();
    }

    async connect(url = 'http://127.0.0.1:9123/rpc', namespace = 'test', database = 'test') {
        try {
            await this.db.connect(url);
            await this.db.use({ namespace, database });
            console.log('Connected to Bioma SurrealDB');
        } catch (error) {
            console.error('Failed to connect to Bioma SurrealDB:', error);
            throw error;
        }
    }

    async close() {
        if (this.db) {
            await this.db.close();
            this.db = undefined;
            console.log('Disconnected from Bioma SurrealDB');
        }
    }

    async sendMessage(actorId, message) {
        const messageId = crypto.randomUUID();
        const frame = {
            id: `message:${messageId}`,
            name: message.constructor.name,
            tx: 'interface:js',
            rx: actorId,
            msg: message
        };

        try {
            await this.db.create('message', frame);
            console.log(`Message sent to Bioma actor ${actorId}`);
            return messageId;
        } catch (error) {
            console.error('Failed to send message to Bioma actor:', error);
            throw error;
        }
    }

    async waitForReply(messageId) {
        const query = `
            LIVE SELECT * FROM reply 
            WHERE id = $replyId
        `;

        try {
            const stream = await this.db.query(query, { replyId: `reply:${messageId}` });
            return new Promise((resolve, reject) => {
                stream.on('data', (data) => {
                    if (data.result && data.result.length > 0) {
                        const reply = data.result[0];
                        console.log(`Received reply from Bioma:`, reply);
                        stream.unsubscribe();
                        resolve(reply.msg);
                    }
                });
                stream.on('error', (error) => {
                    console.error('Error waiting for Bioma reply:', error);
                    reject(error);
                });
            });
        } catch (error) {
            console.error('Failed to set up live query for Bioma reply:', error);
            throw error;
        }
    }

    async sendAndWaitForReply(actorId, message) {
        const messageId = await this.sendMessage(actorId, message);
        return this.waitForReply(messageId);
    }
}

module.exports = BiomaInterface;