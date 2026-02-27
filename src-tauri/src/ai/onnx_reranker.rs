//! BGE-Reranker-v2-m3 交叉重排模型
//! 支持同时输入 [Query, Document] 输出单分数的 Reranker

use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::Tensor;
use rust_tokenizers::tokenizer::{BertTokenizer, Tokenizer, TruncationStrategy};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum RerankerError {
    #[error("模型加载失败: {0}")]
    ModelLoad(String),
    #[error("Tokenizer 加载失败: {0}")]
    TokenizerLoad(String),
    #[error("推理失败: {0}")]
    Inference(String),
}

pub struct OnnxReranker {
    session: Session,
    tokenizer: BertTokenizer,
}

impl OnnxReranker {
    /// 从模型目录加载 Reranker 模型
    /// 需要 model.onnx 和 vocab.txt 文件
    pub fn new(model_dir: &Path) -> Result<Self, RerankerError> {
        let model_path = model_dir.join("reranker.onnx");
        let vocab_path = model_dir.join("reranker_vocab.txt");

        // 如果没有专属命名，回退尝试默认名字
        let model_path = if model_path.exists() { model_path } else { model_dir.join("model.onnx") };
        let vocab_path = if vocab_path.exists() { vocab_path } else { model_dir.join("vocab.txt") };

        // 加载 ONNX 模型
        let session = Session::builder()
            .map_err(|e| RerankerError::ModelLoad(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| RerankerError::ModelLoad(e.to_string()))?
            .commit_from_file(&model_path)
            .map_err(|e| RerankerError::ModelLoad(format!("模型加载失败: {}", e)))?;

        // 加载 BERT tokenizer (BGE-reranker 也是 WordPiece/SentencePiece 架构)
        // XMLi/XLM-Roberta 结构可以使用 BertTokenizer 的改进版，由于目前我们使用的 Tokenizer 是基础版本，
        // 对于 XLM-R 的模型，可能需要特定的 Tokenizer 或者调整，这里我们暂时使用标准模式处理 BGE。
        let tokenizer = BertTokenizer::from_file(&vocab_path, true, true)
            .map_err(|e| RerankerError::TokenizerLoad(format!("无法加载词表: {:?}", e)))?;

        Ok(Self {
            session,
            tokenizer,
        })
    }

    /// 执行重排序，计算 query 和多个文档片段的相关度绝对分数
    /// 返回一个平行于 chunks 数组长度的 f32 数组，每个值为 0~1 的 sigmoid 激活后概率
    pub fn rerank(&mut self, query: &str, chunks: &[&str]) -> Result<Vec<f32>, RerankerError> {
        let mut scores = Vec::with_capacity(chunks.len());
        
        let max_len = 512; // 通常 reranker 支持 512，有的支持更大，安全起见用 512

        for chunk in chunks {
            // 对每个对进行编码: [CLS] Query [SEP] Chunk [SEP]
            // rust_tokenizers 的 encode 接收第二个句子参数来进行这种组装
            let encoding = self.tokenizer.encode(
                query,
                Some(chunk),
                max_len,
                &TruncationStrategy::LongestFirst,
                0, // stride
            );

            let input_ids: Vec<i64> = encoding.token_ids.clone();
            let attention_mask: Vec<i64> = vec![1i64; input_ids.len()];
            // 交叉编码器通常需要 token_type_ids 来区分句 1 和句 2
            let token_type_ids: Vec<i64> = encoding.segment_ids.iter().map(|&x| x as i64).collect();

            let seq_len = input_ids.len();
            let shape = vec![1usize, seq_len];

            // 创建 Tensor
            let input_ids_tensor = Tensor::from_array((shape.clone(), input_ids.into_boxed_slice()))
                .map_err(|e| RerankerError::Inference(format!("Input IDs 创建失败: {}", e)))?;
            let attention_mask_tensor = Tensor::from_array((shape.clone(), attention_mask.into_boxed_slice()))
                .map_err(|e| RerankerError::Inference(format!("Attention Mask 创建失败: {}", e)))?;
            
            // 注意事项：不同 Reranker 模型输入要求可能不同，XLMRoberta 可能不需要 token_type_ids。
            // 稳妥起见我们先尝试包含它
            let has_error = {
                let result = self.session.run(ort::inputs![
                    "input_ids" => input_ids_tensor.clone(),
                    "attention_mask" => attention_mask_tensor.clone(),
                    "token_type_ids" => Tensor::from_array((shape.clone(), token_type_ids.into_boxed_slice()))
                        .map_err(|e| RerankerError::Inference(format!("Token Type IDs 失败: {}", e)))?,
                ]);
                result.is_err()
            };

            let outputs = if has_error {
                // 退避：可能模型不需要 token_type_ids
                self.session.run(ort::inputs![
                    "input_ids" => input_ids_tensor,
                    "attention_mask" => attention_mask_tensor,
                ]).map_err(|e| RerankerError::Inference(format!("退避无 token_type_ids 推理失败: {}", e)))?
            } else {
                self.session.run(ort::inputs![
                    "input_ids" => input_ids_tensor,
                    "attention_mask" => attention_mask_tensor,
                    "token_type_ids" => Tensor::from_array((shape, encoding.segment_ids.iter().map(|&x| x as i64).collect::<Vec<i64>>().into_boxed_slice()))
                        .map_err(|e| RerankerError::Inference(format!("Token Type IDs 失败: {}", e)))?,
                ]).map_err(|e| RerankerError::Inference(format!("包含 token_type_ids 推理失败: {}", e)))?
            };

            // 获取 logits (通常输出的名字叫 logits 或者 output)
            let output_value = outputs
                .get("logits")
                .or_else(|| outputs.get("output"))
                .ok_or_else(|| RerankerError::Inference("未找到 logits 输出".to_string()))?;

            let (_, data) = output_value
                .try_extract_tensor::<f32>()
                .map_err(|e| RerankerError::Inference(format!("输出 Tensor 提取失败: {}", e)))?;

            // Reranker 输出的是 shape [1, 1] 的单个 f32 float logits 值
            let logit = data[0];
            
            // 转化为 0~1 的 sigmoid 概率
            let prob = Self::sigmoid(logit);
            scores.push(prob);
        }

        Ok(scores)
    }

    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }
}
