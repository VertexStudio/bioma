import React from "react";
import { ConfigProvider } from "./contexts/ConfigContext";
import { ChatProvider } from "./contexts/ChatContext";
import Sidebar from "./components/Sidebar/Sidebar.tsx";
import ChatContainer from "./components/Chat/ChatContainer.tsx";
import InputContainer from "./components/Input/InputContainer.tsx";
import "./styles/globalStyles.css";

const App: React.FC = () => {
  return (
    <ConfigProvider>
      <ChatProvider>
        <div className="app-container">
          <Sidebar />
          <main id="main-content">
            <ChatContainer />
            <InputContainer />
          </main>
        </div>
      </ChatProvider>
    </ConfigProvider>
  );
};

export default App;
