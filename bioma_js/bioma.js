const { Surreal, RecordId } = require('surrealdb');
const { ulid } = require('ulid');

class BiomaInterface {
    constructor() {
        this.db = new Surreal();
    }

    async connect(url = 'ws://127.0.0.1:9123', namespace = 'dev', database = 'bioma', user = 'root', password = 'root') {
        try {
            await this.db.connect(url);
            await this.db.signin({
                username: user,
                password: password,
            });
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

    createActorId(id, kind) {
        return {
            id: new RecordId('actor', id),
            kind: kind
        }
    }

    async createActor(id) {
        const actor = await this.db.create('actor', id);
        return actor;
    }

    async sendMessage(tx, rx, name, message) {
        const messageId = ulid();
        const recordId = new RecordId('message', messageId);
        const frame = {
            name: name,
            tx: tx.id,
            rx: rx.id,
            msg: message
        };

        try {
            await this.db.create(recordId, frame);
            console.log(`Message sent to Bioma actor ${rx}`);
            return messageId;
        } catch (error) {
            console.error('Failed to send message to Bioma actor:', error);
            throw error;
        }
    }

    async waitForReply(messageId, maxWaitTime = 10000) {
        const recordId = new RecordId('reply', messageId);
        let waitTime = 0;
        let sleepTime = 100;  // Start with a 100ms sleep

        while (waitTime < maxWaitTime) {
            await new Promise(resolve => setTimeout(resolve, sleepTime));
            waitTime += sleepTime;
            sleepTime = Math.min(sleepTime * 2, 1000);  // Cap at 1 second
            try {
                const reply = await this.db.select(recordId);
                if (reply) {
                    return reply;
                }
            } catch (error) {
                console.error('Error querying for reply:', error);
                throw error;
            }
        }
        throw new Error('Timeout waiting for reply');
    }
}

module.exports = BiomaInterface;