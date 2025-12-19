using UnityEngine;
using UnityEngine.UI;
using UnityEngine.EventSystems;

namespace CodeHotUpdate
{
    /// <summary>
    /// 用途: 模拟《漫威:终极逆转》卡牌的 3D 悬浮视差效果。
    /// 核心逻辑: 结合“RectTransform 物理倾斜”与“Shader 内部视差偏移”，避免纯 Shader 造成的 UV 撕裂。
    /// </summary>
    [DisallowMultipleComponent]
    public sealed class MarvelSnapCardEffect : MonoBehaviour, IPointerEnterHandler, IPointerExitHandler
    {
        #region 配置参数

        [Header("--- 物理运动 (Transform) ---")]
        [Tooltip("是否启用卡牌的物理旋转倾斜")]
        [SerializeField] private bool _enablePhysicalTilt = true;
        
        [Tooltip("最大倾斜角度（度）。建议 5~15 之间，太大会穿模。")]
        [Range(0f, 30f)] 
        [SerializeField] private float _maxTiltAngle = 10f;

        [Tooltip("物理倾斜的平滑时间。越小越灵敏，越大越有'重量感'。")]
        [Range(0.01f, 1f)] 
        [SerializeField] private float _tiltSmoothTime = 0.15f;

        [Header("--- 视觉视差 (Shader) ---")]
        [Tooltip("是否启用 Shader 内部的视差（需要 Shader 支持 _ParallaxOffset 属性）")]
        [SerializeField] private bool _enableShaderParallax = true;

        [Tooltip("Shader 视差强度。控制卡面内部'深处'移动的幅度。建议 0.05~0.2，过大会导致割裂。")]
        [Range(0f, 0.5f)] 
        [SerializeField] private float _parallaxIntensity = 0.1f;
        
        [Tooltip("高光/流光移动强度（需要 Shader 支持 _LightDirection 或类似属性）")]
        [Range(0f, 2f)] 
        [SerializeField] private float _lightSweepIntensity = 1.0f;

        [Header("--- 交互配置 ---")]
        [Tooltip("是否仅当鼠标悬停在卡牌上时才生效？(False = 鼠标在屏幕任何位置都响应)")]
        [SerializeField] private bool _onlyOnHover = false;

        [Tooltip("自动复位：当鼠标移开或不操作时，是否自动回正")]
        [SerializeField] private bool _autoReset = true;

        #endregion

        #region 运行时变量

        private Graphic _targetGraphic;
        private RectTransform _rectTransform;
        private Material _runtimeMaterial;
        private Material _originalMaterial;

        // 平滑阻尼的速度引用变量
        private Vector3 _rotVelocity;
        private Vector2 _parallaxVelocity;
        private Vector2 _currentNormalizedInput; // 当前平滑后的输入 (-1 ~ 1)

        // 目标输入值
        private Vector2 _targetInput; 
        private bool _isHovering;

        #endregion

        #region Shader 属性 ID缓存
        // 假设您的 Shader 使用这些通用属性名，如果不同请修改此处
        private static readonly int Shader_ParallaxOffset = Shader.PropertyToID("_ParallaxOffset");
        private static readonly int Shader_LightDir = Shader.PropertyToID("_LightDir"); // 用于流光
        private static readonly int Shader_HeightStrength = Shader.PropertyToID("_HeightStrength"); // 兼容旧 Shader
        #endregion

        private void Awake()
        {
            _targetGraphic = GetComponent<Graphic>();
            _rectTransform = GetComponent<RectTransform>();
        }

        private void OnEnable()
        {
            EnsureMaterial();
            _currentNormalizedInput = Vector2.zero;
            _targetInput = Vector2.zero;
        }

        private void OnDisable()
        {
            RestoreMaterial();
            // 恢复物理旋转
            if (_rectTransform != null) _rectTransform.localRotation = Quaternion.identity;
        }

        private void OnDestroy()
        {
            RestoreMaterial();
        }

        private void Update()
        {
            // 1. 计算输入
            CalculateInput();

            // 2. 平滑插值 (核心：让动作有惯性，不突兀)
            _currentNormalizedInput = Vector2.SmoothDamp(
                _currentNormalizedInput, 
                _targetInput, 
                ref _parallaxVelocity, 
                _tiltSmoothTime
            );

            // 3. 应用物理旋转 (Tilt)
            ApplyPhysicalTilt();

            // 4. 应用 Shader 视差
            ApplyShaderParallax();
        }

        /// <summary>
        /// 计算鼠标相对于屏幕中心或卡牌中心的归一化坐标 (-1 ~ 1)
        /// </summary>
        private void CalculateInput()
        {
            bool isActive = !_onlyOnHover || _isHovering;

            if (isActive)
            {
                // 获取鼠标相对于屏幕中心的偏移 (-1 到 1)
                float halfWidth = Screen.width * 0.5f;
                float halfHeight = Screen.height * 0.5f;
                Vector3 mousePos = Input.mousePosition;

                float x = Mathf.Clamp((mousePos.x - halfWidth) / halfWidth, -1f, 1f);
                float y = Mathf.Clamp((mousePos.y - halfHeight) / halfHeight, -1f, 1f);

                _targetInput = new Vector2(x, y);
            }
            else if (_autoReset)
            {
                // 归零复位
                _targetInput = Vector2.zero;
            }
            // 如果不复位且不激活，保持最后的状态
        }

        /// <summary>
        /// 应用 RectTransform 的物理旋转
        /// </summary>
        private void ApplyPhysicalTilt()
        {
            if (!_enablePhysicalTilt || _rectTransform == null) return;

            // 鼠标向左(-x)，卡牌应该向左倾斜(y轴旋转负值)？还是向右倾斜？
            // Marvel Snap 风格：鼠标在左，卡牌左边下沉（绕Y轴逆时针）
            
            float rotX = -_currentNormalizedInput.y * _maxTiltAngle; // 绕 X 轴（上下倾斜）
            float rotY = _currentNormalizedInput.x * _maxTiltAngle;  // 绕 Y 轴（左右倾斜）

            Quaternion targetRot = Quaternion.Euler(rotX, rotY, 0f);
            
            _rectTransform.localRotation = targetRot;
        }

        /// <summary>
        /// 将参数传入 Shader
        /// </summary>
        private void ApplyShaderParallax()
        {
            if (_runtimeMaterial == null) return;

            if (_enableShaderParallax)
            {
                // 视差偏移：输入 * 强度
                // 注意：这里我们反向偏移，产生"背景深陷"的感觉
                Vector2 offset = _currentNormalizedInput * _parallaxIntensity * -1f;
                _runtimeMaterial.SetVector(Shader_ParallaxOffset, offset);
                
                // 兼容逻辑：如果你旧的 Shader 用 HeightStrength 控制，这里设死一个值防止乱变
                _runtimeMaterial.SetFloat(Shader_HeightStrength, 1.0f);
            }
            else
            {
                _runtimeMaterial.SetVector(Shader_ParallaxOffset, Vector2.zero);
            }

            // 模拟流光方向：根据倾斜角度改变光照方向
            // 将 2D 输入转换为 3D 光照向量
            Vector4 lightDir = new Vector4(
                -_currentNormalizedInput.x * _lightSweepIntensity, 
                _currentNormalizedInput.y * _lightSweepIntensity, 
                1f, 
                0f
            ).normalized;
            
            _runtimeMaterial.SetVector(Shader_LightDir, lightDir);
        }

        #region 材质管理 (保持原有的私有实例逻辑)

        private void EnsureMaterial()
        {
            if (_targetGraphic == null) return;
            if (_runtimeMaterial != null) return;

            _originalMaterial = _targetGraphic.material;

            // 尝试获取 Shader，如果找不到请修改这里的 Shader 名字
            // 建议使用简单的 Offset Shader，不要用复杂的 POM
            Shader shader = Shader.Find("Custom/ShaderParallaxHeightSculpt233"); 
            
            if (shader == null)
            {
                // 如果找不到你的 Shader，尝试找标准 UI Shader 或报错
                Debug.LogWarning("未找到指定 Shader，请检查名称");
                return;
            }

            _runtimeMaterial = new Material(shader);
            _runtimeMaterial.hideFlags = HideFlags.DontSave;
            _targetGraphic.material = _runtimeMaterial;
        }

        private void RestoreMaterial()
        {
            if (_targetGraphic != null)
            {
                _targetGraphic.material = _originalMaterial;
            }

            if (_runtimeMaterial != null)
            {
                Destroy(_runtimeMaterial);
                _runtimeMaterial = null;
            }
        }

        #endregion

        #region 事件接口
        public void OnPointerEnter(PointerEventData eventData) => _isHovering = true;
        public void OnPointerExit(PointerEventData eventData) => _isHovering = false;
        #endregion
    }
}