(function () {
  function cssColor(name) {
    var probe = document.createElement("span");
    probe.style.color = "var(" + name + ")";
    document.documentElement.appendChild(probe);
    var value = getComputedStyle(probe).color;
    probe.remove();
    return value;
  }

  function clock(stamp) {
    var split = String(stamp).split("T");
    if (split.length < 2) {
      return stamp;
    }
    return split[1].replace(/Z$/, "");
  }

  function draw(canvas) {
    if (typeof window.Chart !== "function") {
      throw new Error("Chart.js is not loaded");
    }
    var samples = JSON.parse(canvas._metricSamples || "[]");
    var existing = window.Chart.getChart(canvas);
    if (existing) {
      existing.destroy();
    }
    if (!samples.length || !canvas.isConnected) {
      return;
    }
    if (canvas.clientWidth < 2 || canvas.clientHeight < 2) {
      if (canvas._metricWait) {
        return;
      }
      canvas._metricWait = true;
      requestAnimationFrame(function () {
        canvas._metricWait = false;
        if (canvas.isConnected && canvas.clientWidth >= 2 && canvas.clientHeight >= 2) {
          draw(canvas);
        }
      });
      return;
    }
    var unit = canvas._metricUnit || "";
    var border = cssColor("--primary");
    var muted = cssColor("--muted-foreground");
    var grid = cssColor("--border");
    var suffix = unit ? " " + unit : "";
    new window.Chart(canvas, {
      type: "line",
      data: {
        labels: samples.map(function (sample) {
          return sample.t;
        }),
        datasets: [
          {
            data: samples.map(function (sample) {
              return sample.v;
            }),
            borderColor: border,
            borderWidth: 2,
            pointRadius: 4,
            pointHoverRadius: 5,
            pointBackgroundColor: cssColor("--background"),
            pointBorderColor: border,
            fill: false,
            tension: 0,
          },
        ],
      },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        animation: false,
        interaction: { mode: "nearest", intersect: false },
        plugins: {
          legend: { display: false },
          tooltip: {
            callbacks: {
              title: function (items) {
                var item = items && items[0];
                return item ? String(item.label) : "";
              },
              label: function (item) {
                return String(item.parsed.y) + suffix;
              },
            },
          },
        },
        scales: {
          x: {
            ticks: {
              color: muted,
              maxTicksLimit: 6,
              callback: function (value) {
                return clock(this.getLabelForValue(value));
              },
            },
            grid: { color: grid },
          },
          y: {
            ticks: { color: muted },
            grid: { color: grid },
          },
        },
      },
    });
  }

  window.renderMetricChart = function (canvas, samplesJson, unit) {
    canvas._metricSamples = samplesJson;
    canvas._metricUnit = unit || "";
    draw(canvas);
  };
})();
