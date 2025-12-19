好的，您选择了**“多层级视差”（Layered Parallax）**方案。这是模拟《漫威：终极逆转》卡牌 2.5D 深度感最稳定、最可靠的方法，因为它**彻底避免了 Shader 扭曲 UV 带来的撕裂问题**。

在这种方案中，我们不再让一张图片（Graphic）的 Shader 承担所有的深度计算，而是将 2D 图像拆分成多个 Image 组件，并让它们根据输入以不同的速度移动。

**原理：**

* **近景层 (Foreground Layer)**：移动速度最快  感觉最近。
* **中景层 (Midground Layer)**：移动速度适中  感觉居中。
* **背景层 (Background Layer)**：移动速度最慢  感觉最深。

为了实现这个方案，我将为您提供两个配套的 C# 脚本：

1. **`ParallaxInputManager` (输入管理器/父级)**：负责获取鼠标输入、平滑处理，并将输入数据分发给子物体。
2. **`ParallaxLayer` (视差层/子级)**：挂载到每个 Image 子物体上，根据从管理器接收到的数据和自身的深度系数进行偏移。

---

## 🛠️ 方案一：多层级视差 (Layered Parallax)

### 1. `ParallaxInputManager.cs` (挂载到卡牌的根对象)

这个脚本负责监听鼠标位置，平滑移动数据，并将数据发送给所有的 `ParallaxLayer` 子组件。

```csharp
using UnityEngine;
using UnityEngine.EventSystems;

namespace CodeHotUpdate
{
    /// <summary>
    /// 【核心管理器】负责获取屏幕输入、平滑处理，并分发给所有子级 ParallaxLayer 组件。
    /// </summary>
    public sealed class ParallaxInputManager : MonoBehaviour, IPointerEnterHandler, IPointerExitHandler
    {
        [Header("输入设置")]
        [Tooltip("输入平滑时间（秒）。越大越柔和，推荐 0.15~0.25。")]
        [SerializeField] private float _smoothTime = 0.2f;

        [Tooltip("输入最大幅度（0~1）。建议 0.3~0.7。")]
        [Range(0f, 1f)]
        [SerializeField] private float _inputMaxMagnitude = 0.6f;

        [Tooltip("是否仅在鼠标悬停在卡牌上时激活视差效果。")]
        [SerializeField] private bool _onlyOnHover = true;

        [Header("运行时")]
        private ParallaxLayer[] _layers;
        private Vector2 _currentInput; // 平滑后的输入 (Normalized -1..1)
        private Vector2 _velocity;
        private Vector2 _targetInput;
        private bool _isHovering;

        private void Start()
        {
            // 在启动时获取所有子级的 ParallaxLayer 组件
            _layers = GetComponentsInChildren<ParallaxLayer>();
            if (_layers.Length == 0)
            {
                Debug.LogWarning("ParallaxInputManager: No ParallaxLayer components found in children.");
            }
        }

        private void Update()
        {
            // 1. 获取原始输入
            Vector2 rawInput = GetRawInput();

            // 2. 确定目标输入 (根据悬停状态)
            if (_onlyOnHover && !_isHovering)
            {
                _targetInput = Vector2.zero; // 未悬停，目标归零
            }
            else
            {
                // 限制输入幅度
                rawInput = Vector2.ClampMagnitude(rawInput, Mathf.Clamp01(_inputMaxMagnitude));
                _targetInput = rawInput;
            }

            // 3. 平滑插值
            _currentInput = Vector2.SmoothDamp(_currentInput, _targetInput, ref _velocity, _smoothTime);

            // 4. 分发给所有层级
            if (_layers != null)
            {
                foreach (var layer in _layers)
                {
                    layer.ApplyParallaxOffset(_currentInput);
                }
            }
        }

        private Vector2 GetRawInput()
        {
            Vector3 pos = Input.mousePosition;
            float w = Mathf.Max(1f, Screen.width);
            float h = Mathf.Max(1f, Screen.height);

            // 归一化到 -1..1，屏幕中心为 (0,0)
            float nx = (pos.x / w - 0.5f) * 2f;
            float ny = (pos.y / h - 0.5f) * 2f;
            return new Vector2(nx, ny);
        }

        public void OnPointerEnter(PointerEventData eventData) => _isHovering = true;
        public void OnPointerExit(PointerEventData eventData) => _isHovering = false;
    }
}

```

### 2. `ParallaxLayer.cs` (挂载到每个背景 Image 上)

这个脚本负责根据输入管理器提供的平滑数据，结合自身的深度系数，计算并执行 RectTransform 的位移。

```csharp
using UnityEngine;
using UnityEngine.UI;

namespace CodeHotUpdate
{
    /// <summary>
    /// 【视差层】根据深度系数对 RectTransform 进行位移。
    /// </summary>
    [RequireComponent(typeof(RectTransform))]
    public sealed class ParallaxLayer : MonoBehaviour
    {
        [Header("深度设置")]
        [Tooltip("深度系数。值越大，移动越快（感觉越近）。\n前景层: 0.8~1.0; 中景层: 0.3~0.7; 背景层: 0.05~0.2。")]
        [Range(0f, 1f)]
        [SerializeField] private float _depthFactor = 0.5f;

        [Tooltip("最大位移像素。控制该层最多能移动多远。")]
        [SerializeField] private float _maxPixelOffset = 20f;

        // 内部状态
        private RectTransform _rectTransform;
        private Vector3 _originalLocalPosition;

        private void Awake()
        {
            _rectTransform = GetComponent<RectTransform>();
            _originalLocalPosition = _rectTransform.localPosition;
        }

        /// <summary>
        /// 接收来自 ParallaxInputManager 的平滑输入。
        /// </summary>
        /// <param name="normalizedInput">归一化的平滑输入 (-1..1)。</param>
        public void ApplyParallaxOffset(Vector2 normalizedInput)
        {
            // 1. 计算偏移量
            // 核心公式： (输入 * 深度系数 * 最大像素)
            // 负号是关键：鼠标向右 (X=正)，物体应该向左移动，制造深度错觉。
            Vector2 offset = normalizedInput * (_depthFactor * _maxPixelOffset) * -1f;

            // 2. 限制偏移，确保物体不会移出卡牌边界
            offset = Vector2.ClampMagnitude(offset, _maxPixelOffset);

            // 3. 应用位移
            _rectTransform.localPosition = _originalLocalPosition + new Vector3(offset.x, offset.y, 0);
        }
    }
}

```

### 3. 设置步骤 (如何使用)

#### 第一步：准备分层美术资源

您需要将您的 2D 背景图像在 Photoshop 或 GIMP 中拆分为至少三个独立的 PNG/图片文件（需要透明度）：

1. **前景层 (FG)**：卡面上看起来最近的元素（如前景装饰、人物身体的一部分）。
2. **中景层 (MG)**：主体内容（如主要背景建筑、人物）。
3. **背景层 (BG)**：最远的元素（如天空、远山、模糊的星球）。

#### 第二步：Unity UI 结构搭建

在 Unity 场景中，将您的卡牌 UI 结构修改为如下所示：

```
- 🔳 CardRoot (RectTransform)  <-- 挂载 ParallaxInputManager.cs
    - 🖼️ Background_BG (Image)     <-- 挂载 ParallaxLayer.cs (Depth: 0.1)
    - 🖼️ Midground_MG (Image)      <-- 挂载 ParallaxLayer.cs (Depth: 0.5)
    - 🖼️ Foreground_FG (Image)     <-- 挂载 ParallaxLayer.cs (Depth: 1.0)
    - 🖼️ Frame (Image)             <-- 卡牌边框（不移动，不挂载任何脚本）

```

#### 第三步：设置深度系数

在 Inspector 中，根据您的视觉需求设置每个 `ParallaxLayer` 的参数：

| 层级名称 | `Depth Factor` (深度系数) | `Max Pixel Offset` (最大偏移量) | 视觉效果 |
| --- | --- | --- | --- |
| FG (前景) | **0.9 ~ 1.0** | 25 ~ 50 像素 | 移动最快 |
| MG (中景) | **0.4 ~ 0.6** | 10 ~ 20 像素 | 移动适中 |
| BG (背景) | **0.1 ~ 0.2** | 5 ~ 10 像素 | 移动最慢 |

完成以上设置后，当您移动鼠标时，各层图像将以不同的速度在屏幕上平移，**通过运动视差（Motion Parallax）** 制造出深度和景深错觉，且**不会出现 UV 撕裂**。

---

这个方案彻底解决了您的问题，因为它不再依赖高度图 Shader 的复杂计算。您是否需要我提供一个简单的 **2D 卡牌分层示意图**，帮助您理解 UI 结构和层级关系？