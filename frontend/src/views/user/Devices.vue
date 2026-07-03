<template>
  <div>
    <div class="toolbar">
      <span class="apple-chip is-primary">共 {{ devices.length }} 台设备</span>
      <el-button type="primary" @click="openDialog()">添加设备</el-button>
    </div>

    <el-table v-loading="loading" :data="devices" class="page-card apple-surface">
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="device_id" label="Device ID" />
      <el-table-column prop="client_id" label="Client ID" />
      <el-table-column label="激活状态" width="120">
        <template #default="{ row }">
          <el-tag :type="row.activated ? 'success' : 'warning'">
            {{ row.activated ? '已激活' : '未激活' }}
          </el-tag>
        </template>
      </el-table-column>
      <el-table-column prop="activation_code" label="激活码" width="120" />
      <el-table-column label="操作" width="180">
        <template #default="{ row }">
          <el-button link type="primary" @click="openDialog(row)">编辑</el-button>
          <el-button link type="danger" @click="remove(row.id)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>

    <el-dialog v-model="dialogVisible" :title="form.id ? '编辑设备' : '添加设备'" width="480px">
      <el-form :model="form" label-position="top">
        <el-form-item label="设备名称">
          <el-input v-model="form.name" />
        </el-form-item>
        <el-form-item label="Device ID">
          <el-input v-model="form.device_id" :disabled="!!form.id" />
        </el-form-item>
        <el-form-item label="Client ID">
          <el-input v-model="form.client_id" />
        </el-form-item>
        <el-form-item label="绑定智能体 ID">
          <el-input v-model.number="form.agent_id" type="number" placeholder="可选" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">取消</el-button>
        <el-button type="primary" @click="save">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import api from '../../utils/api'

const loading = ref(false)
const devices = ref([])
const dialogVisible = ref(false)
const form = reactive({
  id: null,
  name: '',
  device_id: '',
  client_id: '',
  agent_id: null
})

const load = async () => {
  loading.value = true
  try {
    const { data } = await api.get('/user/devices')
    devices.value = data.devices || []
  } finally {
    loading.value = false
  }
}

const openDialog = (row = null) => {
  if (row) {
    Object.assign(form, row)
  } else {
    Object.assign(form, { id: null, name: '', device_id: '', client_id: '', agent_id: null })
  }
  dialogVisible.value = true
}

const save = async () => {
  const payload = {
    name: form.name,
    device_id: form.device_id,
    client_id: form.client_id,
    agent_id: form.agent_id || null
  }
  if (form.id) {
    await api.put(`/user/devices/${form.id}`, payload)
  } else {
    await api.post('/user/devices', payload)
  }
  ElMessage.success('保存成功')
  dialogVisible.value = false
  load()
}

const remove = async (id) => {
  await ElMessageBox.confirm('确定删除该设备？', '提示', { type: 'warning' })
  await api.delete(`/user/devices/${id}`)
  ElMessage.success('已删除')
  load()
}

onMounted(load)
</script>
