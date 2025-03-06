/**
 * Fetches relevant sources based on a query
 * @param {string} query - The search query
 * @param {string} apiEndpoint - The API endpoint to use
 * @returns {Promise<Array>} - Array of source objects
 */
export const fetchRelevantSources = async (query, apiEndpoint) => {
  try {
    const response = await fetch(`${apiEndpoint}/sources`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ query }),
    });

    if (!response.ok) {
      throw new Error(`Failed to fetch sources: ${response.statusText}`);
    }

    const data = await response.json();
    return data.sources || [];
  } catch (error) {
    console.error("Error fetching sources:", error);
    throw error;
  }
};

/**
 * Sends a message to the chat API with optional context
 * @param {string} message - The user message
 * @param {Array} sources - Optional array of sources for context
 * @param {string} apiEndpoint - The API endpoint to use
 * @returns {Promise<Object>} - The response from the API
 */
export const sendChatMessage = async (message, sources = [], apiEndpoint) => {
  try {
    const response = await fetch(`${apiEndpoint}/chat`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        message,
        sources,
      }),
    });

    if (!response.ok) {
      throw new Error(`Failed to send message: ${response.statusText}`);
    }

    return await response.json();
  } catch (error) {
    console.error("Error sending chat message:", error);
    throw error;
  }
};
