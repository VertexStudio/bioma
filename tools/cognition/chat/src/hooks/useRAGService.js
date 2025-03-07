import { useRef, useCallback } from "react";

export function useRAGService({ onResponse, onError, onComplete }) {
  const controllerRef = useRef(null);
  const RAG_ENDPOINT = "http://localhost:5766";

  const sendQuery = useCallback(
    async (endpoint, messages, toolsActors = [], sources = []) => {
      // Abort any previous request
      if (controllerRef.current) {
        controllerRef.current.abort();
      }

      // Create a new AbortController
      controllerRef.current = new AbortController();

      const queryObject = {
        messages,
        model: "llama3.2",
        stream: true,
        ...(toolsActors.length > 0 && { tools_actors: toolsActors }),
        ...(sources.length > 0 && { sources }),
      };

      try {
        const response = await fetch(`${RAG_ENDPOINT}${endpoint}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(queryObject),
          signal: controllerRef.current.signal,
        });

        if (!response.ok) {
          throw new Error(`HTTP error! status: ${response.status}`);
        }

        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";

        while (true) {
          const { value, done } = await reader.read();
          if (done) break;

          const chunk = decoder.decode(value);
          buffer += chunk;

          const lines = buffer.split("\n");
          buffer = lines.pop() || "";

          for (const line of lines) {
            if (!line.trim()) continue;
            try {
              const data = JSON.parse(line);
              onResponse(data);
            } catch (e) {
              console.error("Error processing JSON line:", e);
              console.error("Raw line content:", line);
              onError(`Error processing response: ${e.message}`);
            }
          }
        }

        // Handle completion
        onComplete();
      } catch (error) {
        if (error.name === "AbortError") {
          console.log("Request was aborted");
        } else {
          console.error("Error:", error);
          onError(error.toString());
        }

        onComplete();
      }
    },
    [onResponse, onError, onComplete]
  );

  const abortQuery = useCallback(() => {
    if (controllerRef.current) {
      controllerRef.current.abort();
      controllerRef.current = null;
    }
  }, []);

  const uploadAndIndexSource = useCallback(async (file, sourcePath) => {
    if (!file) {
      throw new Error("No file provided");
    }

    let directoryPath = sourcePath;
    if (directoryPath[0] === "/") {
      directoryPath = directoryPath.slice(1);
    }

    const metadata = {
      path: directoryPath
        ? `uploads/${directoryPath}/${file.name}`
        : `uploads/${file.name}`,
    };

    const formData = new FormData();
    formData.append("file", file);
    formData.append(
      "metadata",
      new Blob([JSON.stringify(metadata)], {
        type: "application/json",
      })
    );

    // Upload the file
    const uploadResponse = await fetch(`${RAG_ENDPOINT}/upload`, {
      method: "POST",
      body: formData,
    });

    if (!uploadResponse.ok) {
      throw new Error(`Upload failed: ${await uploadResponse.text()}`);
    }

    const result = await uploadResponse.json();

    // Index the uploaded file
    const indexQuery = {
      globs: result.paths,
      source: directoryPath ? `/${directoryPath}` : undefined,
      chunk_capacity: {
        start: 500,
        end: 2000,
      },
      chunk_overlap: 200,
      mime_type: file.type,
      summarize: true,
    };

    const indexResponse = await fetch(`${RAG_ENDPOINT}/index`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(indexQuery),
    });

    if (!indexResponse.ok) {
      throw new Error(`Indexing failed: ${await indexResponse.text()}`);
    }

    return await indexResponse.json();
  }, []);

  const fetchAvailableSources = useCallback(async () => {
    try {
      const response = await fetch(`${RAG_ENDPOINT}/sources`, {
        method: "GET",
        headers: { "Content-Type": "application/json" },
      });

      if (!response.ok) {
        throw new Error(
          `Failed to fetch sources: ${response.status} ${response.statusText}`
        );
      }

      // Check if content type is JSON
      const contentType = response.headers.get("content-type");
      if (!contentType || !contentType.includes("application/json")) {
        console.warn("Response is not JSON:", contentType);
        return { sources: [] }; // Return empty sources instead of parsing
      }

      return await response.json();
    } catch (error) {
      console.error("Error in fetchAvailableSources:", error);
      // Return a valid but empty response rather than throwing
      return { sources: [] };
    }
  }, []);

  const deleteSource = useCallback(async (sources) => {
    const response = await fetch(`${RAG_ENDPOINT}/delete_source`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        sources,
        delete_from_disk: false,
      }),
    });

    if (!response.ok) {
      throw new Error(`Failed to delete sources: ${await response.text()}`);
    }

    return await response.json();
  }, []);

  return {
    sendQuery,
    abortQuery,
    uploadAndIndexSource,
    fetchAvailableSources,
    deleteSource,
  };
}
