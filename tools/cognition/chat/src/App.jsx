import React from 'react';
import { ConfigProvider } from './contexts/ConfigContext';
import { ChatProvider } from './contexts/ChatContext';
import Sidebar from './components/Sidebar/Sidebar';
import ChatContainer from './components/Chat/ChatContainer';
import InputContainer from './components/Input/InputContainer';
import './styles/globalStyles.css';

const App = () => {
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