# Sui-cli

---

本项目是一个综合性的多组件系统，旨在通过 AI 代理和命令行界面，实现与 Sui 区块链的智能交互。该系统由一个 Python AI 代理、一个 Rust 命令行客户端以及一个 Rust 驱动的 Sui 工具提供者组成，它们通过 gRPC 和进程间通信紧密协作。

## 🚀 核心架构与组件概览

### 1. 🐍 `agent-py/` - AI 核心与工具协调器

*   **角色**: 作为整个系统的智能大脑，提供 gRPC 服务接口，接收用户请求并生成智能响应。
*   **技术栈**: 
    *   **AI 模型**: 集成 Google Gemini LLM (`gemini-2.5-flash`)，提供强大的语言理解和生成能力。
    *   **代理框架**: 采用 LangGraph 的 ReAct 代理框架，使 AI 能够进行推理、规划并调用外部工具。
    *   **工具集成**: 
        *   **MultiServerMCPClient**: 核心机制，允许 `agent-py` 动态加载并调用由其他 MCP 服务器（特别是 `sui-mcp`）提供的工具。
        *   **RAG (检索增强生成)**: 内置文档问答系统。它从 `docs/` 目录加载 Markdown 和 PDF 文档，利用 HuggingFace Embeddings 和 FAISS 向量数据库构建知识库，并提供 `DocumentationQA` 工具，使 AI 能够回答关于项目、Sui、Tokenomics 等的专业问题。
*   **通信**: 启动一个 gRPC 服务器，监听 `50051` 端口，供 `sui-cli` 连接。
*   **安全性**: 负责安全地接收并传递 `SUI_PRIVATE_KEY` 给 `sui-mcp` 子进程，确保敏感信息不常驻内存。

### 2. 🦀 `sui-cli/` - 用户交互与系统编排

*   **角色**: 用户与整个系统的主要交互界面，负责钱包管理和 `agent-py` 的生命周期管理。
*   **技术栈**: 
    *   **命令行解析**: 使用 `clap` 库处理命令行参数。
    *   **钱包管理**: 提供安全的钱包导入、列表、移除功能。钱包私钥经过密码加密存储，并在运行时解密。
    *   **`agent-py` 启动器**: 负责将 `agent-py` 作为子进程启动，并注入解密后的 `SUI_PRIVATE_KEY` 环境变量。
    *   **gRPC 客户端**: 连接到 `agent-py` 的 gRPC 服务器，发送用户输入并接收 AI 的流式响应。
    *   **TUI (终端用户界面)**: 利用 `crossterm` 和 `ratatui` 构建一个交互式、美观的终端界面，提供流畅的用户体验。
*   **功能**: 用户通过此 CLI 选择钱包、与 AI 代理聊天，并间接触发 Sui 区块链操作。

### 3. 📦 `sui-mcp/` - Sui 区块链工具提供者

*   **角色**: 一个专门为 `agent-py` 提供 Sui 区块链操作工具的 Rust 服务。
*   **技术栈**: 
    *   **MCP 服务器**: 基于 `rmcp` (Rust Multi-Chain Proxy) 框架构建，通过 `stdin/stdout` 与 `agent-py` 进行通信。
    *   **Sui 交互**: 核心是 `SuiService`，负责与 Sui 区块链进行底层交互，包括交易签名和数据查询。
    *   **外部集成**: 集成 `Blockberry API`，用于获取更丰富的资产数据和 DeFi 项目信息。
*   **暴露的工具**: 
    *   `transfer_sui`: SUI 代币转账（支持模拟）。
    *   `get_assets`: 查询指定地址的所有代币和 NFT。
    *   `get_total_value`: 计算指定地址所有资产的美元总价值。
    *   `get_top_defi_projects`: 获取热门 DeFi 项目列表。
    *   `open_project_in_browser`: 在浏览器中打开 Suiscan 项目页面。
*   **安全性**: 接收 `SUI_PRIVATE_KEY` 后立即从环境中移除，遵循安全最佳实践。

## 🌐 整体工作流程

1.  用户通过 `sui-cli` 启动应用，选择并解锁钱包。
2.  `sui-cli` 启动 `agent-py` 进程，并将解锁后的 `SUI_PRIVATE_KEY` 安全地传递给它。
3.  `agent-py` 启动后，会启动 `sui-mcp` 进程，并从 `sui-mcp` 动态加载 Sui 相关的工具。
4.  用户在 `sui-cli` 的 TUI 中与 AI 代理聊天。
5.  `agent-py` 根据用户意图，可能调用其内部的 RAG 工具回答问题，或者调用 `sui-mcp` 提供的 Sui 工具执行区块链操作。
6.  操作结果或 AI 响应通过 gRPC 返回给 `sui-cli`，并在 TUI 中展示给用户。

---

## 📄 许可证

MIT
