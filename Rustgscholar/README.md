# rustgscholar

Google Scholar / OpenAlex 6-Stage Literature Pipeline - Rust Microservice

## 核心功能

强大的学术文献检索与筛选流水线，支持从搜索到元数据补全、排名过滤、全文链接获取及 LLM 智能筛选的全流程。

- **Stage 1: 检索 (Search)**
  - Google Scholar 爬取 (无需 API Key)
  - **OpenAlex API** (推荐, 稳定, 含25+字段: PDF, OA状态, 参考文献, 引用计数等)
- **Stage 2: 元数据补全 (Enrichment)**
  - 通过 Crossref API 补充 DOI、标准期刊名、摘要 (仅 Google Scholar 源需要)
- **Stage 3: 排名过滤 (EasyScholar Ranking)**
  - 按中科院分区 (SCI Q1-Q4)
  - 影响因子 (IF)、JCI 指数过滤
  - 预警期刊识别 (Top/各级预警)
- **Stage 4: 全文与摘要补充 (Semantic Scholar)**
  - 基于 DOI 的批量查询
  - 获取**干净摘要** (相比 Google Scholar 截断版更完整)
  - 获取 **OA PDF 下载链接**
  - 获取 TLDR 摘要 (AI 生成的一句话总结)
- **Stage 5: 统一输出 (Unified)**
  - 合并所有阶段数据，生成统一的 CSV 格式
  - 摘要优先级：Semantic Scholar > OpenAlex/Crossref
  - 日期标准化：优先使用精确日期 (YYYY-MM-DD)，降级使用年份
- **Stage 6: LLM 智能筛选 (LLM Filter)** *(可选)*
  - 基于大语言模型的相关性分类
  - 支持 OpenAI API 兼容接口 (如 DeepSeek, Qwen 等)
  - 并发批量处理，Token 用量追踪
  - 分类结果：relevant / irrelevant / uncertain

## 安装

```bash
cargo build --release
```

## 快速开始

### 推荐流程：OpenAlex + EasyScholar + Semantic Scholar

```bash
# 搜索 "deep learning"，获取最近 3 年，SCI Q1 区，影响因子 > 5 的论文
cargo run --release -- search "deep learning" \
    --source openalex \    # 使用 OpenAlex 源 (自带 DOI, 更快)
    --pages 1-5 \          # 抓取前 5 页 (约 1000 条)
    --ylo 2022 \           # 2022 年至今
    --easyscholar-key "YOUR_KEY" \
    --sciif 5.0 \          # 影响因子 >= 5.0
    --sci Q1               # SCI 一区
```

### 启用 LLM 智能筛选 (Stage 6)

```bash
cargo run --release -- search "transformer model" \
    --source openalex \
    --pages 1-3 \
    --easyscholar-key "YOUR_KEY" \
    --sci Q1 \
    --llm-base-url "https://api.deepseek.com/v1" \
    --llm-key "YOUR_LLM_KEY" \
    --llm-model "deepseek-chat" \
    --filter-help "研究主题：注意力机制在NLP中的应用"
```

## CLI 模式详解

### OpenAlex 模式 (推荐)
速度快，无需代理，自带 25+ 种元数据字段。

```bash
cargo run --release -- search "transformer model" --source openalex --pages 1
```

**OpenAlex 输出字段 (1_openalex.csv):**
- 基础信息: `title`, `author`, `year`, `publication_date`, `venue`, `doi`
- 链接: `article_url`, **`pdf_url`**, `oa_url`
- 状态: **`is_oa`** (是否开源), `oa_status` (Gold/Green/Bronze)
- 类型: `work_type`, `source_type` (journal/repository)
- 内容: `keywords`, `primary_topic`, `snippet` (摘要片段)
- 引用: `referenced_works` (参考文献ID列表), `related_works` (相关文献ID列表)

### Google Scholar 模式
适合必须使用 Google 搜索算法的场景。需配合 Crossref (Stage 2) 使用。

```bash
# 建议配置代理
cargo run --release -- search "neural network" --proxy "http://127.0.0.1:7890"
```

### 完整过滤与增强示例

```bash
cargo run --release -- search "large language models" \
    --source openalex \
    --pages 1-3 \
    --easyscholar-key "YOUR_KEY" \
    --sciif 10.0 \         # 高影响力期刊
    --sci Q1
```

## CLI 参数说明

| 基本参数 | 说明 |
|----------|------|
| `keyword` | 搜索关键词（必需） |
| `--source` | 数据源：`openalex` (推荐) 或 `gscholar` |
| `--pages` | 页码范围，如 `1` 或 `1-10` (OpenAlex 每页 200 条) |
| `--ylo` | 年份下限（如 2023） |
| `--output` | 输出目录（默认：`./output`） |

| EasyScholar 过滤 | 说明 |
|------------------|------|
| `--easyscholar-key` | EasyScholar API Key (必需，用于 Stage 3) |
| `--sciif` | 影响因子筛选 (>= 值) |
| `--jci` | JCI 指数筛选 (>= 值) |
| `--sci` | SCI 分区筛选 (如 "Q1", "Q1,Q2") |

| LLM 筛选参数 | 说明 |
|--------------|------|
| `--llm-base-url` | OpenAI 兼容 API 地址 (如 `https://api.deepseek.com/v1`) |
| `--llm-key` | LLM API 密钥 |
| `--llm-model` | 模型名称 (如 `deepseek-chat`, `gpt-4o-mini`) |
| `--filter-help` | 筛选关键词/主题描述 (帮助 LLM 判断相关性) |

> **获取 Key**: 
> - EasyScholar: 访问 [EasyScholar 官网](https://www.easyscholar.cc/) 个人中心 -> 开放接口
> - LLM API: 根据所选模型服务商获取

## 输出文件结构

```
output/{timestamp}_{keyword}/
├── 1_openalex.csv        # Stage 1: 包含所有字段的原始数据
├── 3_easyscholar.csv     # Stage 3: 经过排名过滤的高质量论文
├── 4_semanticscholar.csv # Stage 4: Semantic Scholar 增强数据
├── 5_unified.csv         # Stage 5: 统一格式的最终数据
└── 6_llm_filtered.csv    # Stage 6: LLM 筛选结果 (可选)
```

**5_unified.csv 字段:**
- `title`, `author`, `date`: 基本信息
- `doi`, `article_url`, `pdf_url`: 链接信息
- `abstract_text`: 完整摘要 (优先 Semantic Scholar)
- `tldr`: AI 一句话总结
- `journal`, `if_score`, `jci_score`, `sci_partition`: 期刊排名信息

**6_llm_filtered.csv 字段:**
- 包含所有 `5_unified.csv` 字段
- `relevance`: 相关性判断 (`relevant` / `irrelevant` / `uncertain`)
- `reason`: LLM 给出的判断理由

## 项目结构

```
src/
├── main.rs            # 6-Stage 流水线调度
├── openalex.rs        # OpenAlex API (Polite Pool, 25+ 字段提取)
├── semanticscholar.rs # Semantic Scholar API (Batch DOI 查询)
├── rankings.rs        # EasyScholar API (缓存优化: 聚合查询)
├── unified.rs         # 统一输出生成 (Stage 5)
├── llm_filter.rs      # LLM 相关性筛选 (Stage 6)
├── prompts/           # LLM 提示词模板
│   ├── mod.rs
│   └── relevance_filter.rs
├── gscholar.rs        # Google Scholar 爬虫
├── crossref.rs        # Crossref API
├── error.rs           # 错误处理
├── lib.rs             # 模块导出
└── cookies.rs         # Cookie 管理
```

## TODO

- [ ] **语义检索**: 基于 Stage 5 的摘要进行向量化搜索
- [ ] **持久化缓存**: EasyScholar 24小时本地缓存 (Redis/JSON)
- [ ] **自动下载**: 根据 `pdf_url` 自动下载论文 PDF
- [ ] **批量处理优化**: LLM API 批量请求支持

## License

MIT
