<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";

interface Option {
  value: string;
  label: string;
}

const props = defineProps<{
  modelValue: string[];
  options: Option[];
  disabled?: boolean;
  placeholder?: string;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();

const root = ref<HTMLElement>();
const open = ref(false);
const selectedLabels = computed(() =>
  props.options
    .filter((option) => props.modelValue.includes(option.value))
    .map((option) => option.label),
);
const summary = computed(() => {
  if (!selectedLabels.value.length) return props.placeholder ?? "未选择";
  if (selectedLabels.value.length <= 2) return selectedLabels.value.join("、");
  return `已选择 ${selectedLabels.value.length} 项`;
});

function toggleOption(value: string) {
  const next = props.modelValue.includes(value)
    ? props.modelValue.filter((item) => item !== value)
    : [...props.modelValue, value];
  emit("update:modelValue", next);
}

function closeOnOutsideClick(event: MouseEvent) {
  if (!root.value?.contains(event.target as Node)) open.value = false;
}

onMounted(() => document.addEventListener("mousedown", closeOnOutsideClick));
onUnmounted(() => document.removeEventListener("mousedown", closeOnOutsideClick));
</script>

<template>
  <div ref="root" class="multi-select" :class="{ open, disabled }">
    <button type="button" class="multi-trigger" :disabled="disabled" @click="open = !open">
      <span>{{ summary }}</span>
      <i>▾</i>
    </button>
    <div v-if="open" class="multi-menu">
      <label v-for="option in options" :key="option.value" class="multi-option">
        <input
          type="checkbox"
          :checked="modelValue.includes(option.value)"
          @change="toggleOption(option.value)"
        />
        <span>{{ option.label }}</span>
        <code>{{ option.value }}</code>
      </label>
    </div>
  </div>
</template>

<style scoped>
.multi-select { position: relative; }
.multi-trigger { width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 9px 10px; color: #e9eef5; background: #0b0f13; border: 1px solid #2a353e; border-radius: 5px; text-align: left; }
.multi-trigger span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.multi-trigger i { color: #71808c; font-style: normal; transition: transform .16s; }
.open .multi-trigger { border-color: #4bb68f; }
.open .multi-trigger i { transform: rotate(180deg); }
.multi-menu { position: absolute; z-index: 20; left: 0; right: 0; top: calc(100% + 5px); max-height: 260px; overflow: auto; padding: 5px; background: #0c1115; border: 1px solid #34414a; border-radius: 6px; box-shadow: 0 14px 36px #000a; }
.multi-option { display: grid; grid-template-columns: 18px 1fr auto; align-items: center; gap: 8px; padding: 8px; color: #c6d0d8; border-radius: 4px; cursor: pointer; }
.multi-option:hover { background: #172129; }
.multi-option input { accent-color: #4bc99b; }
.multi-option code { color: #70808d; font: 10px ui-monospace, "Cascadia Mono", monospace; }
.disabled { opacity: .5; }
</style>
