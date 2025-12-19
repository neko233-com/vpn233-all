using UnityEngine;
using UnityEngine.UI;

namespace CodeHotUpdate
{
    /// <summary>
    /// 【目标：2D 模拟 3D 景深卡牌效果 - 优化版】
    /// 关键点:
    /// 1. 自动创建私有材质实例，避免影响共享材质。
    /// 2. 对输入做 SmoothDamp 平滑，避免突兀的“割裂线/闪烁”。
    /// 3. 精简逻辑，专注于核心的高度图/视差效果控制。
    /// 注意事项: 目标必须是 UnityEngine.UI.Graphic；OnDisable/OnDestroy 会恢复原材质。
    /// </summary>
    [DisallowMultipleComponent]
    public sealed class UIHeightSculptOptimized : MonoBehaviour
    {
        #region 序列化字段

        [Header("目标 Graphic")]
        [Tooltip("要应用雕刻的目标 Graphic（Image/RawImage/Text 等，不填则自动取本物体上的 Graphic）")]
        [SerializeField]
        private Graphic _target;

        [Header("视差输入控制")]
        [Tooltip("是否启用以屏幕中心为原点的鼠标偏移输入（归一化 -1..1）。")]
        [SerializeField]
        private bool _useMouseCenterOffsetInput = true;

        [Tooltip("输入平滑时间（秒）。越大越柔和，越小越跟手。建议 0.1 ~ 0.3。")]
        [SerializeField]
        [Range(0.01f, 1f)]
        private float _inputSmoothTime = 0.18f;

        [Tooltip("视差偏移的最大幅度（0~1）。越小越稳定，越大越立体。建议 0.1~0.4。")]
        [Range(0f, 1f)]
        [SerializeField]
        private float _parallaxMaxMagnitude = 0.3f;

        [Tooltip("视差输入缩放（对最终传入 Shader 的 _ParallaxOffset 生效）")]
        [Range(0f, 2f)]
        [SerializeField]
        private float _inputScale = 1.0f;

        [Header("高度图 & 浮雕光照")]
        [Tooltip("高度图（约定：越深色=高度越高，越浅色=高度越低）")]
        [SerializeField]
        private Texture2D _heightMap;

        [Tooltip("高度强度/深度（对应 Shader _HeightStrength）")]
        [Range(0f, 1f)]
        [SerializeField]
        private float _heightStrength = 0.25f;

        [Tooltip("法线强度（越大越硬朗，模拟凹凸感；过大容易脏）")]
        [Range(0f, 5f)]
        [SerializeField]
        private float _normalStrength = 1.5f;

        [Header("POM/景深优化（可选）")]
        [Tooltip("是否启用 POM/UV 位移。建议关闭以追求稳定（使用 Parallax Mapping），或小幅度开启。")]
        [SerializeField]
        private bool _enableParallaxUVShift = false;

        [Tooltip("POM UV 位移尺度/深度（启用时生效）。极小值，建议 0.005 ~ 0.01。")]
        [Range(0f, 0.05f)]
        private float _heightUVScale = 0.008f;

        // 【注意】Toon Steps、Rim 等二次元光照参数保留，但不再是造成割裂的主因。

        #endregion

        #region Shader IDs (假定 Shader 沿用相同的属性名)

        private static readonly int Shader_HeightMap = Shader.PropertyToID("_HeightMap");
        private static readonly int Shader_HeightStrength = Shader.PropertyToID("_HeightStrength");
        private static readonly int Shader_ParallaxOffset = Shader.PropertyToID("_ParallaxOffset");
        
        // 简化后的 POM/UV 偏移控制
        private static readonly int Shader_EnablePOM = Shader.PropertyToID("_EnablePOM");
        private static readonly int Shader_HeightUVScale = Shader.PropertyToID("_HeightUVScale");
        
        // 浮雕光照参数
        private static readonly int Shader_NormalStrength = Shader.PropertyToID("_NormalStrength");
        
        // 旧的 FocusPoint, SculptStrength 等参数如果 Shader 不需要，将不再使用。

        #endregion

        #region 运行时字段

        private Material _originalMaterial;
        private Material _runtimeMaterial;

        private Vector2 _inputSmoothed;
        private Vector2 _inputVelocity;

        #endregion

        private void Reset()
        {
            _target = GetComponent<Graphic>();
        }

        private void Awake()
        {
            if (_target == null)
            {
                _target = GetComponent<Graphic>();
            }
        }

        private void OnEnable()
        {
            EnsureMaterial();
            _inputSmoothed = Vector2.zero;
            _inputVelocity = Vector2.zero;
        }

        private void Update()
        {
            if (_target == null || _runtimeMaterial == null)
            {
                EnsureMaterial();
                if (_runtimeMaterial == null) return;
            }

            Vector2 raw = GetRawInput();
            
            // 1. 限制最大幅度，确保输入在合理范围内
            raw = Vector2.ClampMagnitude(raw, Mathf.Clamp01(_parallaxMaxMagnitude));
            
            // 2. 应用缩放
            raw *= Mathf.Max(0f, _inputScale);

            // 3. 平滑处理
            float smooth = Mathf.Max(0.01f, _inputSmoothTime);
            _inputSmoothed = Vector2.SmoothDamp(_inputSmoothed, raw, ref _inputVelocity, smooth);

            // 4. 应用到材质
            ApplyToMaterial(_runtimeMaterial, _inputSmoothed);
        }

        private void OnDisable()
        {
            RestoreMaterial();
        }

        private void OnDestroy()
        {
            RestoreMaterial();
        }

        // ------------------------- 核心逻辑方法 -------------------------

        private Vector2 GetRawInput()
        {
            if (_useMouseCenterOffsetInput)
            {
                Vector3 pos = Input.mousePosition;
                float w = Mathf.Max(1f, Screen.width);
                float h = Mathf.Max(1f, Screen.height);

                // 归一化到 -1..1，屏幕中心为 (0,0)
                float nx = (pos.x / w - 0.5f) * 2f;
                float ny = (pos.y / h - 0.5f) * 2f;
                return new Vector2(nx, ny);
            }

            return Vector2.zero;
        }

        private void EnsureMaterial()
        {
            if (_target == null)
            {
                return;
            }

            if (_runtimeMaterial != null)
            {
                return;
            }
            
            // 备份原材质
            _originalMaterial = _target.material;

            // 假定您使用的 Shader 名称是这个或类似，请确保路径正确
            Shader shader = Shader.Find("Custom/ShaderParallaxHeightSculpt233");
            if (shader == null)
            {
                Debug.LogError("Shader 'Custom/ShaderParallaxHeightSculpt233' Not Found! Please check the shader path.");
                return;
            }

            // 创建私有材质实例
            _runtimeMaterial = new Material(shader);
            _runtimeMaterial.hideFlags = HideFlags.DontSave;
            _target.material = _runtimeMaterial;

            // 立即应用一次参数
            ApplyToMaterial(_runtimeMaterial, Vector2.zero);
        }

        private void RestoreMaterial()
        {
            if (_target != null)
            {
                // 恢复原始材质
                _target.material = _originalMaterial;
            }

            if (_runtimeMaterial != null)
            {
                // 销毁运行时材质实例
                Destroy(_runtimeMaterial);
                _runtimeMaterial = null;
            }

            _originalMaterial = null;
        }

        private void ApplyToMaterial(Material mat, Vector2 input)
        {
            if (mat == null)
            {
                return;
            }

            // 【核心】视差输入（控制主贴图/法线的偏移）
            mat.SetVector(Shader_ParallaxOffset, input);

            // 高度图及强度
            mat.SetFloat(Shader_HeightStrength, Mathf.Clamp01(_heightStrength));
            mat.SetTexture(Shader_HeightMap, _heightMap);

            // 浮雕光照强度
            mat.SetFloat(Shader_NormalStrength, Mathf.Clamp(_normalStrength, 0f, 5f));
            
            // POM/UV 偏移控制
            mat.SetFloat(Shader_EnablePOM, _enableParallaxUVShift ? 1f : 0f);
            mat.SetFloat(Shader_HeightUVScale, Mathf.Clamp(_heightUVScale, 0f, 0.05f));
            
            // 注意：其他 Toon/Rim/Focus 等参数请自行在 Inspector 中调整或根据需要添加到此方法中。
        }
    }
}