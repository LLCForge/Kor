// ÜRETİLDİ: scripts/provider-logos.ts, kaynak assets/providers/*.svg. Elle düzenlemeyin.
import abliteration from '../../../../assets/providers/abliteration.svg'
import abliteration_color from '../../../../assets/providers/abliteration-color.svg'
import aionlabs from '../../../../assets/providers/aionlabs.svg'
import aionlabs_color from '../../../../assets/providers/aionlabs-color.svg'
import alibabacloud from '../../../../assets/providers/alibabacloud.svg'
import alibabacloud_color from '../../../../assets/providers/alibabacloud-color.svg'
import anthropic from '../../../../assets/providers/anthropic.svg'
import arliai from '../../../../assets/providers/arliai.svg'
import aws from '../../../../assets/providers/aws.svg'
import aws_color from '../../../../assets/providers/aws-color.svg'
import azure from '../../../../assets/providers/azure.svg'
import azure_color from '../../../../assets/providers/azure-color.svg'
import azureai from '../../../../assets/providers/azureai.svg'
import azureai_color from '../../../../assets/providers/azureai-color.svg'
import bedrock from '../../../../assets/providers/bedrock.svg'
import bedrock_color from '../../../../assets/providers/bedrock-color.svg'
import cerebras from '../../../../assets/providers/cerebras.svg'
import cerebras_color from '../../../../assets/providers/cerebras-color.svg'
import chatglm from '../../../../assets/providers/chatglm.svg'
import chatglm_color from '../../../../assets/providers/chatglm-color.svg'
import claude from '../../../../assets/providers/claude.svg'
import claude_color from '../../../../assets/providers/claude-color.svg'
import cohere from '../../../../assets/providers/cohere.svg'
import cohere_color from '../../../../assets/providers/cohere-color.svg'
import deepseek from '../../../../assets/providers/deepseek.svg'
import deepseek_color from '../../../../assets/providers/deepseek-color.svg'
import fal from '../../../../assets/providers/fal.svg'
import fal_color from '../../../../assets/providers/fal-color.svg'
import fireworks from '../../../../assets/providers/fireworks.svg'
import fireworks_color from '../../../../assets/providers/fireworks-color.svg'
import flux from '../../../../assets/providers/flux.svg'
import gemini from '../../../../assets/providers/gemini.svg'
import gemini_color from '../../../../assets/providers/gemini-color.svg'
import github from '../../../../assets/providers/github.svg'
import google from '../../../../assets/providers/google.svg'
import google_color from '../../../../assets/providers/google-color.svg'
import grok from '../../../../assets/providers/grok.svg'
import groq from '../../../../assets/providers/groq.svg'
import huggingface from '../../../../assets/providers/huggingface.svg'
import huggingface_color from '../../../../assets/providers/huggingface-color.svg'
import kimi from '../../../../assets/providers/kimi.svg'
import kimi_color from '../../../../assets/providers/kimi-color.svg'
import kling from '../../../../assets/providers/kling.svg'
import kling_color from '../../../../assets/providers/kling-color.svg'
import llamacpp from '../../../../assets/providers/llamacpp.svg'
import lmstudio from '../../../../assets/providers/lmstudio.svg'
import localai from '../../../../assets/providers/localai.svg'
import localai_color from '../../../../assets/providers/localai-color.svg'
import mancer from '../../../../assets/providers/mancer.svg'
import mancer_color from '../../../../assets/providers/mancer-color.svg'
import meta from '../../../../assets/providers/meta.svg'
import meta_color from '../../../../assets/providers/meta-color.svg'
import mistral from '../../../../assets/providers/mistral.svg'
import mistral_color from '../../../../assets/providers/mistral-color.svg'
import moonshot from '../../../../assets/providers/moonshot.svg'
import nvidia from '../../../../assets/providers/nvidia.svg'
import nvidia_color from '../../../../assets/providers/nvidia-color.svg'
import ollama from '../../../../assets/providers/ollama.svg'
import openai from '../../../../assets/providers/openai.svg'
import openrouter from '../../../../assets/providers/openrouter.svg'
import openrouter_color from '../../../../assets/providers/openrouter-color.svg'
import perplexity from '../../../../assets/providers/perplexity.svg'
import perplexity_color from '../../../../assets/providers/perplexity-color.svg'
import qwen from '../../../../assets/providers/qwen.svg'
import qwen_color from '../../../../assets/providers/qwen-color.svg'
import sambanova from '../../../../assets/providers/sambanova.svg'
import sambanova_color from '../../../../assets/providers/sambanova-color.svg'
import together from '../../../../assets/providers/together.svg'
import together_color from '../../../../assets/providers/together-color.svg'
import uncensoredllm from '../../../../assets/providers/uncensoredllm.svg'
import uncensoredllm_color from '../../../../assets/providers/uncensoredllm-color.svg'
import venice from '../../../../assets/providers/venice.svg'
import venice_color from '../../../../assets/providers/venice-color.svg'
import vertexai from '../../../../assets/providers/vertexai.svg'
import vertexai_color from '../../../../assets/providers/vertexai-color.svg'
import vllm from '../../../../assets/providers/vllm.svg'
import vllm_color from '../../../../assets/providers/vllm-color.svg'
import xai from '../../../../assets/providers/xai.svg'
import zai from '../../../../assets/providers/zai.svg'
import zhipu from '../../../../assets/providers/zhipu.svg'
import zhipu_color from '../../../../assets/providers/zhipu-color.svg'

/** Logo kimliği → tek renk (currentColor) ve tam renkli SVG metni (renkli varyant yoksa tek renk). */
export const PROVIDER_LOGOS: Readonly<Record<string, { mono: string; color: string }>> = {
  "abliteration": { mono: abliteration, color: abliteration_color },
  "aionlabs": { mono: aionlabs, color: aionlabs_color },
  "alibabacloud": { mono: alibabacloud, color: alibabacloud_color },
  "anthropic": { mono: anthropic, color: anthropic },
  "arliai": { mono: arliai, color: arliai },
  "aws": { mono: aws, color: aws_color },
  "azure": { mono: azure, color: azure_color },
  "azureai": { mono: azureai, color: azureai_color },
  "bedrock": { mono: bedrock, color: bedrock_color },
  "cerebras": { mono: cerebras, color: cerebras_color },
  "chatglm": { mono: chatglm, color: chatglm_color },
  "claude": { mono: claude, color: claude_color },
  "cohere": { mono: cohere, color: cohere_color },
  "deepseek": { mono: deepseek, color: deepseek_color },
  "fal": { mono: fal, color: fal_color },
  "fireworks": { mono: fireworks, color: fireworks_color },
  "flux": { mono: flux, color: flux },
  "gemini": { mono: gemini, color: gemini_color },
  "github": { mono: github, color: github },
  "google": { mono: google, color: google_color },
  "grok": { mono: grok, color: grok },
  "groq": { mono: groq, color: groq },
  "huggingface": { mono: huggingface, color: huggingface_color },
  "kimi": { mono: kimi, color: kimi_color },
  "kling": { mono: kling, color: kling_color },
  "llamacpp": { mono: llamacpp, color: llamacpp },
  "lmstudio": { mono: lmstudio, color: lmstudio },
  "localai": { mono: localai, color: localai_color },
  "mancer": { mono: mancer, color: mancer_color },
  "meta": { mono: meta, color: meta_color },
  "mistral": { mono: mistral, color: mistral_color },
  "moonshot": { mono: moonshot, color: moonshot },
  "nvidia": { mono: nvidia, color: nvidia_color },
  "ollama": { mono: ollama, color: ollama },
  "openai": { mono: openai, color: openai },
  "openrouter": { mono: openrouter, color: openrouter_color },
  "perplexity": { mono: perplexity, color: perplexity_color },
  "qwen": { mono: qwen, color: qwen_color },
  "sambanova": { mono: sambanova, color: sambanova_color },
  "together": { mono: together, color: together_color },
  "uncensoredllm": { mono: uncensoredllm, color: uncensoredllm_color },
  "venice": { mono: venice, color: venice_color },
  "vertexai": { mono: vertexai, color: vertexai_color },
  "vllm": { mono: vllm, color: vllm_color },
  "xai": { mono: xai, color: xai },
  "zai": { mono: zai, color: zai },
  "zhipu": { mono: zhipu, color: zhipu_color },
}
