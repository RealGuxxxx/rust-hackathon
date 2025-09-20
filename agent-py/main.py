# agent_with_mcp.py
import grpc
import asyncio
import os
import json
import traceback
import sys
from dotenv import load_dotenv

import agent_pb2
import agent_pb2_grpc

# LLM
from langchain_google_genai import ChatGoogleGenerativeAI

# LangChain agent (使用 create_react_agent)
from langgraph.prebuilt import create_react_agent

# LangChain core history (用于会话管理)
from langchain_core.chat_history import InMemoryChatMessageHistory
from langchain_core.messages import HumanMessage, AIMessage

# MCP adapters
from langchain_mcp_adapters.client import MultiServerMCPClient

# RAG - 新增的导入
from langchain_community.document_loaders import DirectoryLoader, UnstructuredMarkdownLoader, PyPDFLoader
from langchain_community.vectorstores import FAISS
from langchain_huggingface import HuggingFaceEmbeddings
from langchain.text_splitter import RecursiveCharacterTextSplitter
from langchain.chains import RetrievalQA
from langchain.tools import Tool

# ========= 环境变量 =========
load_dotenv()
google_api_key = os.getenv("GOOGLE_API_KEY")
if not google_api_key:
    raise ValueError("请在 .env 文件中设置 GOOGLE_API_KEY")

# ========= LLM =========
llm = ChatGoogleGenerativeAI(
    model="gemini-2.5-flash",
    google_api_key=google_api_key,
    temperature=0.2,
)

# ========= RAG (Retrieval-Augmented Generation) =========

# Get the absolute path to the 'docs' directory relative to this script's location.
# This makes file access robust, regardless of the current working directory.
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
DOCS_PATH = os.path.join(SCRIPT_DIR, 'docs')

# Path for the cached FAISS index
FAISS_INDEX_PATH = os.path.join(SCRIPT_DIR, "faiss_index")

def create_rag_retriever(docs_path=DOCS_PATH, model_name="sentence-transformers/all-MiniLM-L6-v2"):
    """
    Creates a RAG retriever. It loads the index from disk if it exists,
    otherwise it builds it and saves it to disk for future runs.
    """
    embeddings = HuggingFaceEmbeddings(model_name=model_name)

    # Check if the index already exists on disk
    if os.path.exists(FAISS_INDEX_PATH):
        print(f"[RAG] Loading existing FAISS index from '{FAISS_INDEX_PATH}'...")
        # FAISS.load_local requires allow_dangerous_deserialization=True
        # This is safe here because we are creating the index file ourselves.
        vector_store = FAISS.load_local(FAISS_INDEX_PATH, embeddings, allow_dangerous_deserialization=True)
        print("[RAG] FAISS index loaded successfully.")
        return vector_store.as_retriever()

    # If index doesn't exist, build it from scratch
    print(f"[RAG] No existing index found. Building new index from '{docs_path}'...")
    print("[RAG] This will take a while on the first run...")

    md_loader = DirectoryLoader(
        docs_path,
        glob="**/*.md",
        loader_cls=UnstructuredMarkdownLoader,
        show_progress=True,
        use_multithreading=False
    )
    pdf_loader = DirectoryLoader(
        docs_path,
        glob="**/*.pdf",
        loader_cls=PyPDFLoader,
        show_progress=True,
        use_multithreading=False
    )

    print("[RAG] Loading documents...")
    documents = md_loader.load() + pdf_loader.load()

    if not documents:
        print("[RAG] No documents found to build index.")
        return None

    print(f"[RAG] Loaded {len(documents)} documents. Splitting into chunks...")
    text_splitter = RecursiveCharacterTextSplitter(chunk_size=1000, chunk_overlap=200)
    texts = text_splitter.split_documents(documents)
    print(f"[RAG] Split into {len(texts)} chunks. Creating embeddings and FAISS index...")

    vector_store = FAISS.from_documents(texts, embeddings)
    print("[RAG] FAISS index created successfully.")

    # Save the newly created index to disk for future runs
    print(f"[RAG] Saving new index to '{FAISS_INDEX_PATH}'...")
    vector_store.save_local(FAISS_INDEX_PATH)
    print("[RAG] Index saved successfully.")

    return vector_store.as_retriever()


# ========= 会话历史 =========
store = {}


def get_session_history(session_id: str):
    if session_id not in store:
        store[session_id] = InMemoryChatMessageHistory()
    return store[session_id]


# ========= gRPC Servicer =========
class AgentService(agent_pb2_grpc.AgentServiceServicer):
    def __init__(self, agent, session_getter, mcp_client):
        self.agent = agent
        self.get_session_history = session_getter
        self.mcp_client = mcp_client

    async def Chat(self, request, context):
        user_input = request.message
        session_id = "test-session"
        print(f"[Agent] 收到 Chat (session={session_id}): {user_input}")

        session_history = self.get_session_history(session_id)

        try:
            # 将历史消息和当前用户输入构造成代理所需的格式
            messages = []
            # 添加历史消息
            for msg in session_history.messages:
                if msg.type == "human":
                    messages.append(HumanMessage(content=msg.content))
                elif msg.type == "ai":
                    messages.append(AIMessage(content=msg.content))
            # 添加当前用户消息
            messages.append(HumanMessage(content=user_input))

            # 调用代理
            agent_response = await self.agent.ainvoke({"messages": messages})

            # 代理的响应是一个字典，其中 "messages" 包含了新的消息列表
            new_messages = agent_response["messages"]

            # 将新的消息添加到历史记录中
            for msg in new_messages[len(messages):]:  # 只添加新生成的消息
                if isinstance(msg, HumanMessage):
                    session_history.add_user_message(msg.content)
                elif isinstance(msg, AIMessage):
                    session_history.add_ai_message(msg.content)

                    # 安全地将内容转换为字符串
                    content_str = ""
                    if isinstance(msg.content, str):
                        content_str = msg.content
                    elif isinstance(msg.content, (list, dict)):
                        # 如果是结构化数据，转换为 JSON 字符串
                        content_str = json.dumps(msg.content, ensure_ascii=False, indent=2)
                    else:
                        # 其他类型，强制转为字符串
                        content_str = str(msg.content)

                    # 按字符流式输出，确保兼容性
                    for char in content_str:
                        yield agent_pb2.ChatResponse(content=char)

            yield agent_pb2.ChatResponse(content="[STREAM_END]")

        except Exception as e:
            error_msg = f"Chat 处理失败: {str(e)}"
            print(f"[ERROR] {error_msg}", file=sys.stderr)
            traceback.print_exc(file=sys.stderr)
            await context.abort(grpc.StatusCode.UNKNOWN, error_msg)

    async def ExecuteAction(self, request, context):
        print(f"[Agent] 收到 ExecuteAction: {request.action} {request.params}")
        action_result = {
            "executed_action": request.action,
            "params": json.loads(request.params) if request.params else {},
            "status": "ok"
        }
        return agent_pb2.ActionResponse(
            success=True,
            result=json.dumps(action_result)
        )


# ========= 主服务启动 =========
async def serve():
    # --- 从环境变量安全地读取私钥 ---
    private_key = os.getenv("SUI_PRIVATE_KEY")
    if not private_key:
        print("[ERROR] SUI_PRIVATE_KEY environment variable not set. This service should be launched by sui-cli.", file=sys.stderr)
        sys.exit(1)
    
    # --- 立即从当前环境移除，减少暴露 ---
    try:
        del os.environ["SUI_PRIVATE_KEY"]
    except KeyError:
        pass # 如果键不存在，也无妨

    mcp_servers = {
        "sui_tools": {
            "command": "/path/sui-mcp",
            "args": [],
            "transport": "stdio",
            # --- 将私钥作为环境变量传递给 sui-mcp 子进程 ---
            "env": {
                "SUI_PRIVATE_KEY": private_key,
                "RUST_LOG": "info"
            }
        }
    }

    client = MultiServerMCPClient(mcp_servers)
    print("[MCP] MultiServerMCPClient 已初始化，开始加载工具...")
    tools = await client.get_tools()
    print(f"[MCP] 已加载 {len(tools)} 个工具: {[getattr(t, 'name', str(t)) for t in tools]}")

    # --- RAG 工具创建 ---
    all_tools = list(tools)
    rag_retriever = create_rag_retriever()
    if rag_retriever:
        rag_qa_chain = RetrievalQA.from_chain_type(
            llm=llm,
            chain_type="stuff",
            retriever=rag_retriever,
            return_source_documents=True
        )
        
        def run_rag_qa(query: str):
            result = rag_qa_chain({"query": query})
            # 可以在这里格式化输出，比如只返回答案或包含来源
            return result["result"]

        rag_tool = Tool(
            name="DocumentationQA",
            func=run_rag_qa,
            description="当需要回答关于项目文档、SUI、Tokenomics 或相关技术细节的问题时使用。输入应该是一个完整的问题。"
        )
        all_tools.append(rag_tool)
        print(f"[RAG] 'DocumentationQA' 工具已创建并添加。")
    else:
        print("[RAG] 由于未能创建检索器，'DocumentationQA' 工具未添加。")
    # --- RAG 工具创建结束 ---

    # 创建代理
    agent = create_react_agent(llm, all_tools)

    server = grpc.aio.server()
    servicer = AgentService(agent, get_session_history, client)
    agent_pb2_grpc.add_AgentServiceServicer_to_server(servicer, server)
    server.add_insecure_port("[::]:50051")
    await server.start()
    print("[Agent] async gRPC 服务启动，监听端口 50051")
    await server.wait_for_termination()


if __name__ == "__main__":
    try:
        asyncio.run(serve())
    except KeyboardInterrupt:
        print("退出中...")