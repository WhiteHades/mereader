import pandas as pd
import numpy as np
import plotly.graph_objects as go
import plotly.express as px
from plotly.subplots import make_subplots
from pathlib import Path
from scipy import stats
import warnings

warnings.filterwarnings('ignore')

# configuration
RAW_PATH = 'evaluation_results_multi_model_combined_cleaned.csv'
out_dir = Path('research_paper_analysis_plotly')
out_dir.mkdir(exist_ok=True)
fig_dir = out_dir / 'figs'
fig_dir.mkdir(exist_ok=True)

print("[INFO] Loading and preprocessing data...")
df = pd.read_csv(RAW_PATH)
df.columns = [c.strip() for c in df.columns]

# apply scaling transformations
df['response_time_log'] = np.log10(df['response_time'].clip(lower=1e-6))
df['retrieval_effectiveness_log'] = np.log10(df['retrieval_effectiveness'].clip(lower=1e-6))
df['unique_word_ratio_log'] = np.log10(df['unique_word_ratio'].clip(lower=1e-6))
df['bleu_scaled'] = df['bleu'] * 1000

regular_df = df[df['evaluation_type'] == 'regular'].copy()
spoiler_df = df[df['evaluation_type'] == 'spoiler'].copy()

print(f"[INFO] Loaded {len(df)} total rows ({len(regular_df)} regular, {len(spoiler_df)} spoiler)")

# 1: Precision Recall Plane
def create_precision_recall_plane():
    """Precision Recall plane showing model performance"""
    
    model_means = regular_df.groupby('model').agg({
        'bert_precision': 'mean',
        'bert_recall': 'mean',
        'bert_f1': 'mean'
    }).reset_index()
    
    fig = go.Figure()
    
    f1_values = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
    for f1 in f1_values:
        r_values = np.linspace(f1/2 + 0.01, 0.99, 100)
        p_values = f1 * r_values / (2 * r_values - f1)
        
        valid_mask = (p_values > 0) & (p_values < 1)
        r_values = r_values[valid_mask]
        p_values = p_values[valid_mask]
        
        fig.add_trace(go.Scatter(
            x=r_values,
            y=p_values,
            mode='lines',
            line=dict(dash='dash', color='lightgray', width=1),
            name=f'F1={f1}',
            showlegend=False,
            hoverinfo='skip'
        ))
    
    unique_models = model_means['model'].unique()
    discrete_colors = px.colors.qualitative.Set1
    model_color_map = {model: discrete_colors[i % len(discrete_colors)] for i, model in enumerate(unique_models)}
    
    # add individual traces for each model to create legend
    for i, (_, row) in enumerate(model_means.iterrows()):
        fig.add_trace(go.Scatter(
            x=[row['bert_recall']],
            y=[row['bert_precision']],
            mode='markers',
            marker=dict(
                size=(row['bert_f1'] * 20) + 10,
                color=model_color_map[row['model']],
                line=dict(width=2, color='black')
            ),
            hovertemplate=f'<b>{row["model"]}</b><br>Recall: %{{x:.3f}}<br>Precision: %{{y:.3f}}<br>F1: {row["bert_f1"]:.3f}<extra></extra>',
            name=row['model'],
            showlegend=True
        ))
    
    fig.update_layout(
        xaxis_title='Recall',
        yaxis_title='Precision',
        width=1000,
        height=800,
        template='plotly_white',
        xaxis=dict(
            range=[0.5, 0.8],
            title=dict(font=dict(size=32)),
            tickfont=dict(size=24)
        ),
        yaxis=dict(
            range=[0.4, 0.8],
            title=dict(font=dict(size=32)),
            tickfont=dict(size=24)
        ),
        legend=dict(
            font=dict(size=24),
            x=0.02,
            y=0.98
        ),
        margin=dict(l=80, r=40, t=40, b=80)
    )
    
    return fig

# 3: Speed Quality Efficiency & Pareto Frontier
def create_speed_quality_efficiency():
    """Speed-quality efficiency analysis with Pareto frontier"""
    
    # calculate model means
    model_means = regular_df.groupby('model').agg({
        'response_time': 'mean',
        'bert_f1': 'mean',
        'context_count': 'mean'
    }).reset_index()
    
    # calculate efficiency
    model_means['efficiency'] = model_means['bert_f1'] / model_means['response_time']
    
    fig = go.Figure()
    
    unique_models = model_means['model'].unique()
    discrete_colors = px.colors.qualitative.Set1
    model_color_map = {model: discrete_colors[i % len(discrete_colors)] for i, model in enumerate(unique_models)}
    
    # add individual traces for each model to create legend
    for i, (_, row) in enumerate(model_means.iterrows()):
        fig.add_trace(go.Scatter(
            x=[row['response_time']],
            y=[row['bert_f1']],
            mode='markers',
            marker=dict(
                size=(row['efficiency'] * 50) + 10,
                color=model_color_map[row['model']],
                line=dict(width=2, color='black')
            ),
            hovertemplate=f'<b>{row["model"]}</b><br>Response Time: %{{x:.2f}}s<br>BERT F1: %{{y:.3f}}<br>Efficiency: {row["efficiency"]:.3f}<extra></extra>',
            name=row['model'],
            showlegend=True
        ))
    
    # pareto frontier
    sorted_models = model_means.sort_values(['response_time', 'bert_f1'], ascending=[True, False])
    frontier_models = []
    best_quality = -np.inf
    
    for _, row in sorted_models.iterrows():
        if row['bert_f1'] > best_quality:
            frontier_models.append(row)
            best_quality = row['bert_f1']
    
    if frontier_models:
        frontier_df = pd.DataFrame(frontier_models)
        fig.add_trace(go.Scatter(
            x=frontier_df['response_time'],
            y=frontier_df['bert_f1'],
            mode='lines+markers',
            line=dict(dash='dash', color='red', width=3),
            marker=dict(symbol='diamond', size=15, color='red'),
            name='Pareto Frontier',
            hovertemplate='Pareto Optimal<extra></extra>'
        ))
        
        # add annotations for Pareto frontier models
        for _, row in frontier_df.iterrows():
            fig.add_annotation(
                x=row['response_time'],
                y=row['bert_f1'],
                text=f"* {row['model']}",
                showarrow=True,
                arrowhead=2,
                arrowsize=1,
                arrowwidth=2,
                arrowcolor='red',
                ax=20,
                ay=-30,
                bgcolor='yellow',
                bordercolor='red',
                borderwidth=2,
                font=dict(size=20, color='black')
            )
    
    # add median reference lines
    x_med = model_means['response_time'].median()
    y_med = model_means['bert_f1'].median()
    fig.add_hline(y=y_med, line=dict(color='gray', dash='dot'))
    fig.add_vline(x=x_med, line=dict(color='gray', dash='dot'))

    fig.update_layout(
        xaxis_title='Response Time (seconds)',
        yaxis_title='BERT F1 Score',
        width=1000,
        height=700,
        template='plotly_white',
        xaxis=dict(
            title=dict(font=dict(size=32)),
            tickfont=dict(size=24)
        ),
        yaxis=dict(
            title=dict(font=dict(size=32)),
            tickfont=dict(size=24)
        ),
        legend=dict(
            font=dict(size=24),
            x=0.02,
            y=0.10
        ),
        margin=dict(l=100, r=60, t=40, b=100)
    )
    
    return fig

# 4: robustness across reading progress
def create_reading_progress_robustness():
    """BERT metrics across reading stages (context count)"""
    
    progress_data = regular_df.groupby(['model', 'progress_stage']).agg({
        'bert_precision': 'mean',
        'bert_recall': 'mean',
        'bert_f1': 'mean'
    }).reset_index()

    # fallback if progress is effectively constant
    progress_unique = progress_data['progress_stage'].nunique()
    use_progress = progress_unique > 3
    if not use_progress:
        # bin by context_count as longitudinal proxy using qcut (handles duplicate edges)
        tmp = regular_df.copy()
        try:
            ctx_bin = pd.qcut(tmp['context_count'], q=3, labels=['low', 'mid', 'high'], duplicates='drop')
        except Exception:
            # last resort rank first to break ties
            ctx_bin = pd.qcut(tmp['context_count'].rank(method='first'), q=3, labels=['low', 'mid', 'high'], duplicates='drop')

        # if too few bins, coarsen or fallback to single bin
        if hasattr(ctx_bin, 'cat') and len(ctx_bin.cat.categories) >= 2:
            tmp['context_bin'] = ctx_bin
        else:
            tmp['context_bin'] = pd.Series(['all'] * len(tmp), index=tmp.index, dtype='category')

        progress_data = tmp.groupby(['model', 'context_bin']).agg({
            'bert_precision': 'mean', 'bert_recall': 'mean', 'bert_f1': 'mean'
        }).reset_index().rename(columns={'context_bin': 'progress_stage'})
    
    # create subplots
    fig = make_subplots(
        rows=1, cols=3,
        subplot_titles=('BERT Precision', 'BERT Recall', 'BERT F1'),
        specs=[[{"type": "scatter"}, {"type": "scatter"}, {"type": "scatter"}]]
    )
    
    # update subplot titles font size
    for annotation in fig.layout.annotations:
        annotation.font.size = 28
    
    models = progress_data['model'].unique()
    colors = px.colors.qualitative.Set1
    
    for i, model in enumerate(models):
        model_data = progress_data[progress_data['model'] == model].sort_values('progress_stage')
        
        fig.add_trace(go.Scatter(
            x=model_data['progress_stage'],
            y=model_data['bert_precision'],
            mode='lines+markers',
            name=f'{model} (Precision)',
            line=dict(color=colors[i % len(colors)], width=2),
            marker=dict(size=8),
            showlegend=False
        ), row=1, col=1)
        
        fig.add_trace(go.Scatter(
            x=model_data['progress_stage'],
            y=model_data['bert_recall'],
            mode='lines+markers',
            name=f'{model} (Recall)',
            line=dict(color=colors[i % len(colors)], width=2),
            marker=dict(size=8),
            showlegend=False
        ), row=1, col=2)
        
        fig.add_trace(go.Scatter(
            x=model_data['progress_stage'],
            y=model_data['bert_f1'],
            mode='lines+markers',
            name=model,
            line=dict(color=colors[i % len(colors)], width=2),
            marker=dict(size=8),
            showlegend=True
        ), row=1, col=3)
    
    fig.update_layout(
        height=600,
        width=1500,
        template='plotly_white',
        legend=dict(
            font=dict(size=24),
            x=1.02,
            y=0.5
        )
    )
    
    x_title = "Progress Stage (%)" if use_progress else "Context Count Bin"
    for col in [1, 2, 3]:
        fig.update_xaxes(
            title_text=x_title,
            title_font=dict(size=28),
            tickfont=dict(size=22),
            row=1, col=col
        )
        fig.update_yaxes(
            title_text="Score",
            title_font=dict(size=28),
            tickfont=dict(size=22),
            row=1, col=col
        )
    
    return fig

# 5: Error Analysis
def create_error_profile_analysis():
    """Two panel figure showing error types and their distribution"""
    
    # filter out 'none' errors for analysis
    filtered_df = regular_df[regular_df['error_type'] != 'none'].copy()
    
    # panel 1: error type distribution by model
    error_counts = filtered_df.groupby(['model', 'error_type']).size().unstack(fill_value=0)
    models_with_errors = error_counts.index[error_counts.sum(axis=1) > 0]
    error_counts = error_counts.loc[models_with_errors]
    error_percent = (error_counts.T / error_counts.sum(axis=1)).T.fillna(0) * 100
    
    # panel 2: error count by model and type
    error_counts_abs = error_counts.copy()
    
    fig = make_subplots(
        rows=1, cols=2,
        subplot_titles=('Error Type Distribution by Model (% Share)', 'Error Count by Model and Type'),
        specs=[[{"type": "bar"}, {"type": "bar"}]]
    )
    
    for annotation in fig.layout.annotations:
        annotation.font.size = 18
    
    # panel 1: error distribution
    for error_type in error_percent.columns:
        fig.add_trace(go.Bar(
            x=error_percent.index,
            y=error_percent[error_type],
            name=error_type,
            opacity=0.8
        ), row=1, col=1)
    
    # panel 2: error counts
    for error_type in error_counts_abs.columns:
        fig.add_trace(go.Bar(
            x=error_counts_abs.index,
            y=error_counts_abs[error_type],
            name=error_type,
            opacity=0.8,
            showlegend=False
        ), row=1, col=2)
    
    fig.update_layout(
        height=600,
        width=1500,
        template='plotly_white',
        xaxis=dict(
            tickangle=45,
            title=dict(font=dict(size=28)),
            tickfont=dict(size=22)
        ),
        xaxis2=dict(
            tickangle=45,
            title=dict(font=dict(size=28)),
            tickfont=dict(size=22)
        ),
        legend=dict(
            font=dict(size=22)
        )
    )
    
    fig.update_xaxes(
        title_text="Model",
        title_font=dict(size=24),
        tickfont=dict(size=18),
        row=1, col=1
    )
    fig.update_yaxes(
        title_text="% of Errors",
        title_font=dict(size=24),
        tickfont=dict(size=18),
        row=1, col=1
    )
    fig.update_xaxes(
        title_text="Model",
        title_font=dict(size=24),
        tickfont=dict(size=18),
        row=1, col=2
    )
    fig.update_yaxes(
        title_text="Error Count",
        title_font=dict(size=24),
        tickfont=dict(size=18),
        row=1, col=2
    )
    
    return fig

# 6: Answer Quality Radar
def create_holistic_quality_radar():
    """Radar chart showing answer quality metrics"""
    
    quality_metrics = [
        'relevance', 'helpfulness', 'coherence', 'contextual_fidelity', 
        'factual_grounding', 'instruction_following', 'paas_score'
    ]
    
    model_means = regular_df.groupby('model')[quality_metrics].mean()
    
    for col in quality_metrics:
        min_val, max_val = model_means[col].min(), model_means[col].max()
        if max_val > min_val:
            model_means[col] = (model_means[col] - min_val) / (max_val - min_val)
    
    fig = go.Figure()
    
    models = model_means.index.tolist()
    
    distinct_colors = [
        '#FF0000',
        '#0000FF',
        '#00FF00',
        '#FF8000',
        '#8000FF',
        '#00FFFF',
        '#FF00FF',
        '#FFFF00',
        '#000000',
        '#808080',
        '#8B4513',
        '#FF69B4'
    ]
    
    for i, model in enumerate(models):
        values = [model_means.loc[model, col] for col in quality_metrics]
        labels = [col.replace('_', ' ').title() for col in quality_metrics]
        
        fig.add_trace(go.Scatterpolar(
            r=values,
            theta=labels,
            fill='none',
            name=model,
            line=dict(color=distinct_colors[i % len(distinct_colors)], width=3),
            opacity=0.8,
            marker=dict(size=6)
        ))
    
    fig.update_layout(
        polar=dict(
            radialaxis=dict(
                visible=True,
                range=[0, 1],
                tickfont=dict(size=22)
            ),
            angularaxis=dict(
                tickfont=dict(size=24)
            )
        ),
        width=800,
        height=800,
        template='plotly_white',
        legend=dict(
            font=dict(size=24),
            x=0.85,
            y=-0.15
        )
    )
    
    return fig

def main():
    print("[INFO] Generating research-focused Plotly visualizations...")
    
    plots = {
        '01_precision_recall_plane': create_precision_recall_plane(),
        '03_speed_quality_efficiency': create_speed_quality_efficiency(),
        '04_reading_progress_robustness': create_reading_progress_robustness(),
        '05_error_profile_analysis': create_error_profile_analysis(),
        '06_holistic_quality_radar': create_holistic_quality_radar()
    }
    
    for name, fig in plots.items():
        fig.write_image(fig_dir / f'{name}.png', width=1200, height=800, scale=3)
        print(f"[DONE] Saved {name}.png")
    
    print(f"[DONE] Analysis complete! Results saved to {fig_dir}")
    print(f"[INFO] Generated {len(plots)} research-focused visualizations")

if __name__ == '__main__':
    main()