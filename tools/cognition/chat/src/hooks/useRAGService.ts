import { useRef, useCallback } from "react";

interface Source {
  path: string;
  active: boolean;
  isLocal?: boolean;
}

interface SourceItem {
  source: string;
  uri: string;
}

interface SourcesResponse {
  sources: SourceItem[];
}

interface Tool {
  id: string;
  actor: string;
  active: boolean;
}

interface QueryParams {
  message: string;
  tools: Tool[];
  sources: Source[];
  think: boolean;
  signal: AbortSignal;
}

interface RAGServiceOptions {
  onResponse?: (data: any) => void;
  onError?: (error: Error | string) => void;
  onComplete?: () => void;
}

interface RAGServiceResult {
  sendQuery: (params: QueryParams) => Promise<void>;
  abortQuery: () => void;
  fetchAvailableSources: () => Promise<SourcesResponse>;
  uploadAndIndexSource: (file: File, path: string) => Promise<any>;
  deleteSource: (paths: string[]) => Promise<any>;
}

export function useRAGService(
  options: RAGServiceOptions = {}
): RAGServiceResult {
  const { onResponse, onError, onComplete } = options;
  const controllerRef = useRef<AbortController | null>(null);
  const RAG_ENDPOINT = "http://localhost:5766";

  const abortQuery = useCallback(() => {
    if (controllerRef.current) {
      controllerRef.current.abort();
      controllerRef.current = null;
    }
  }, []);

  const sendQuery = useCallback(
    async (params: QueryParams): Promise<void> => {
      // Abort any previous request
      if (controllerRef.current) {
        controllerRef.current.abort();
      }

      // Create a new AbortController
      controllerRef.current = new AbortController();

      const queryObject = {
        messages: [{ role: "user", content: params.message }],
        model: "llama3.2",
        stream: true,
        ...(params.tools.length > 0 && {
          tools_actors: params.tools.map((t) => t.actor),
        }),
        ...(params.sources.length > 0 && {
          sources: params.sources.map((s) => s.path),
        }),
        ...(params.think && { think: true }),
      };

      try {
        const response = await fetch(`${RAG_ENDPOINT}/chat`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(queryObject),
          signal: params.signal || controllerRef.current.signal,
        });

        if (!response.ok) {
          throw new Error(`HTTP error! status: ${response.status}`);
        }

        if (!response.body) {
          throw new Error("Response body is null");
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
            if (line.trim() === "") continue;

            try {
              const data = JSON.parse(line);
              if (onResponse) onResponse(data);
            } catch (err) {
              console.error("Error parsing SSE:", err);
              console.log("Problematic line:", line);
              if (onError)
                onError(`Error processing response: ${(err as Error).message}`);
            }
          }
        }

        if (onComplete) onComplete();
      } catch (error) {
        if ((error as Error).name === "AbortError") {
          console.log("Request was aborted");
        } else {
          console.error("Error in RAG service:", error);
          if (onError) onError(error as Error);
        }

        if (onComplete) onComplete();
      }
    },
    [onResponse, onError, onComplete, RAG_ENDPOINT]
  );

  const uploadAndIndexSource = useCallback(
    async (file: File, sourcePath: string): Promise<any> => {
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
    },
    [RAG_ENDPOINT]
  );

  const fetchAvailableSources =
    useCallback(async (): Promise<SourcesResponse> => {
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
    }, [RAG_ENDPOINT]);

  const deleteSource = useCallback(
    async (sources: string[]): Promise<any> => {
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
    },
    [RAG_ENDPOINT]
  );

  return {
    sendQuery,
    abortQuery,
    uploadAndIndexSource,
    fetchAvailableSources,
    deleteSource,
  };
}
